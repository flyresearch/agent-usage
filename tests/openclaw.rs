use agent_usage::*;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};
fn db(p: &Path) -> Connection {
    let c = Connection::open(p).unwrap();
    c.execute_batch("CREATE TABLE transcript_events(session_id TEXT,seq INTEGER,event_json TEXT,event_zstd BLOB,event_utf8_bytes INTEGER);CREATE TABLE trajectory_runtime_events(session_id TEXT,run_id TEXT,event_json TEXT);").unwrap();
    c
}
fn config(p: &Path) -> Config {
    Config {
        schema_version: 1,
        host_id: "fixture".into(),
        sources: vec![Source {
            id: "openclaw".into(),
            adapter: "openclaw-sqlite".into(),
            paths: vec![p.into()],
        }],
        model_aliases: BTreeMap::from([("synthetic".into(), "model-a".into())]),
    }
}
fn message(api: &str) -> Value {
    json!({"type":"message","message":{"role":"assistant","api":api,"provider":"synthetic-provider","model":"synthetic","timestamp":1767225600000i64,"usage":{"input":10,"output":20,"cacheRead":30,"cacheWrite":40,"reasoningTokens":5,"totalTokens":100}}})
}
fn add(c: &Connection, seq: i64, v: &Value, compressed: bool) {
    let raw = serde_json::to_vec(v).unwrap();
    if compressed {
        let b = zstd::bulk::compress(&raw, 1).unwrap();
        c.execute(
            "INSERT INTO transcript_events VALUES('s',?1,NULL,?2,?3)",
            params![seq, b, raw.len() as i64],
        )
        .unwrap();
    } else {
        c.execute(
            "INSERT INTO transcript_events VALUES('s',?1,?2,NULL,?3)",
            params![
                seq,
                String::from_utf8(raw.clone()).unwrap(),
                raw.len() as i64
            ],
        )
        .unwrap();
    }
}
fn runtime(c: &Connection, thread: &str) {
    c.execute(
        "INSERT INTO trajectory_runtime_events VALUES('s','run',?1)",
        [
            json!({"type":"model.completed","data":{"threadId":thread,"usage":{"input":999}}})
                .to_string(),
        ],
    )
    .unwrap();
}
fn backend(root: &Path, c: &mut Config) {
    let logs = root.join("backend");
    std::fs::create_dir(&logs).unwrap();
    let events = [
        json!({"type":"session_meta","payload":{"id":"thread"}}),
        json!({"type":"turn_context","payload":{"model":"synthetic"}}),
        json!({"timestamp":"2026-01-01T00:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":2,"total_tokens":110}}}}),
    ];
    std::fs::write(
        logs.join("a.jsonl"),
        events.iter().map(|e| format!("{e}\n")).collect::<String>(),
    )
    .unwrap();
    c.sources.push(Source {
        id: "backend".into(),
        adapter: "openclaw-codex-jsonl".into(),
        paths: vec![logs],
    });
}
#[test]
fn json_and_zstd_use_the_same_mutually_exclusive_mapping() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = db(&p);
    add(&c, 1, &message("google-generative-ai"), false);
    add(&c, 2, &message("openai-completions"), true);
    let s = collect(&config(&p), None).unwrap();
    assert_eq!(
        s.totals,
        Tokens {
            input: 20,
            output: 30,
            reasoning: 10,
            cache_read: 60,
            cache_write: 80
        }
    );
    assert_eq!(s.rows.len(), 2);
    assert!(s.rows.iter().all(|r| r.messages == MessageCount::one()));
    assert_eq!(s.rows[0].agent, "openclaw");
    assert!(s
        .rows
        .iter()
        .all(|r| r.provider.as_ref().unwrap().starts_with("provider-")));
    assert_eq!(output::summary(&s).unwrap()["known_total_tokens"], 200);
}
#[test]
fn runtime_linked_mirror_and_backend_count_once_even_if_sqlite_configured_first() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = db(&p);
    runtime(&c, "thread");
    let mut m = message("openai-responses");
    m["message"]["__openclaw"] = json!({"runId":"run"});
    add(&c, 1, &m, true);
    let mut cfg = config(&p);
    backend(d.path(), &mut cfg);
    let cache = d.path().join("cache.json");
    for _ in 0..2 {
        let s = collect(&cfg, Some(&cache)).unwrap();
        assert_eq!(s.totals.total().unwrap(), 110);
        assert_eq!(s.rows[0].agent, "openclaw");
        assert_eq!(s.rows[0].backend.as_deref(), Some("codex"));
        assert_eq!(s.sources[0].coverage.mirrored_usage_records, 1);
        assert_eq!(s.sources[0].coverage.runtime_usage_records, 1);
        assert_eq!(s.sources[0].coverage.missing_backend_threads, 0);
    }
}
#[test]
fn absent_backend_is_a_gap_never_a_zero_success_claim() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = db(&p);
    runtime(&c, "missing-thread");
    let mut m = message("openai-responses");
    m["message"]["__openclaw"] = json!({"runId":"run"});
    add(&c, 1, &m, false);
    let s = collect(&config(&p), None).unwrap();
    assert_eq!(s.sources[0].coverage.missing_backend_threads, 1);
    assert_eq!(s.sources[0].coverage.unclassified_usage_records, 1);
    assert_eq!(s.sources[0].status, "partial");
    assert!(!s.coverage_complete);
}
#[test]
fn model_and_current_window_never_determine_historical_backend() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = db(&p);
    c.execute_batch("CREATE TABLE session_windows(agent_harness_id TEXT);INSERT INTO session_windows VALUES('codex');").unwrap();
    add(&c, 1, &message("openai-responses"), false);
    add(&c, 2, &message("google-generative-ai"), false);
    let s = collect(&config(&p), None).unwrap();
    assert_eq!(s.totals.total().unwrap(), 100);
    assert_eq!(s.sources[0].coverage.unclassified_usage_records, 1);
}
#[test]
fn replay_identity_deduplicates_and_conflict_fails() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = db(&p);
    let mut m = message("openai-completions");
    m["message"]["idempotencyKey"] = json!("call");
    add(&c, 1, &m, false);
    add(&c, 2, &m, true);
    assert_eq!(
        collect(&config(&p), None).unwrap().totals.total().unwrap(),
        100
    );
    m["message"]["usage"]["input"] = json!(11);
    m["message"]["usage"]["totalTokens"] = json!(101);
    add(&c, 3, &m, false);
    assert_eq!(
        collect(&config(&p), None).unwrap_err().0,
        "conflicting_message_replay"
    );
}
#[test]
fn invalid_usage_cannot_hide_behind_unknown_backend() {
    for (key, value) in [
        ("input", json!(-1)),
        ("output", json!(null)),
        ("reasoningTokens", json!(21)),
        ("totalTokens", json!(999)),
    ] {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.db");
        let c = db(&p);
        let mut m = message("unknown");
        m["message"]["usage"][key] = value;
        add(&c, 1, &m, false);
        assert!(collect(&config(&p), None).is_err());
    }
}
#[test]
fn corrupt_compressed_event_and_size_limit_fail() {
    for (blob, size) in [
        (vec![1, 2, 3], 100),
        (zstd::bulk::compress(b"{}", 1).unwrap(), 3),
        (vec![1], 17 * 1024 * 1024),
    ] {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.db");
        let c = db(&p);
        c.execute(
            "INSERT INTO transcript_events VALUES('s',1,NULL,?1,?2)",
            params![blob, size],
        )
        .unwrap();
        assert!(collect(&config(&p), None).is_err());
    }
}
#[test]
fn runtime_linkage_is_scoped_to_session_and_conflicts_fail() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = db(&p);
    runtime(&c, "thread");
    c.execute(
        "UPDATE trajectory_runtime_events SET session_id='other'",
        [],
    )
    .unwrap();
    let mut m = message("openai-completions");
    m["message"]["__openclaw"] = json!({"runId":"run"});
    add(&c, 1, &m, false);
    let mut cfg = config(&p);
    backend(d.path(), &mut cfg);
    assert_eq!(collect(&cfg, None).unwrap().totals.total().unwrap(), 210);
    runtime(&c, "thread");
    runtime(&c, "contradiction");
    assert_eq!(
        collect(&cfg, None).unwrap_err().0,
        "conflicting_backend_linkage"
    );
}
#[test]
fn wal_updates_are_visible_and_uncommitted_records_are_not() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = db(&p);
    c.execute_batch("PRAGMA journal_mode=WAL;PRAGMA wal_autocheckpoint=0;BEGIN IMMEDIATE;")
        .unwrap();
    add(&c, 1, &message("anthropic-messages"), true);
    let before = std::fs::read(&p).unwrap();
    assert_eq!(
        collect(&config(&p), None).unwrap().totals.total().unwrap(),
        0
    );
    c.execute_batch("COMMIT").unwrap();
    let wal = std::fs::read(d.path().join("a.db-wal")).unwrap();
    assert_eq!(
        collect(&config(&p), None).unwrap().totals.total().unwrap(),
        100
    );
    assert_eq!(std::fs::read(&p).unwrap(), before);
    assert_eq!(std::fs::read(d.path().join("a.db-wal")).unwrap(), wal);
}

#[test]
fn cross_database_replay_cannot_change_provider_or_backend() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a.db");
    let b = d.path().join("b.db");
    let ca = db(&a);
    let cb = db(&b);
    let mut m = message("openai-completions");
    m["message"]["idempotencyKey"] = json!("call");
    add(&ca, 1, &m, false);
    m["message"]["provider"] = json!("different");
    add(&cb, 1, &m, false);
    let mut cfg = config(&a);
    cfg.sources[0].paths.push(b);
    assert_eq!(collect(&cfg, None).unwrap_err().0, "conflicting_replay");
}

#[test]
fn state_destinations_cannot_overwrite_sqlite_journals() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = db(&p);
    c.execute_batch("PRAGMA journal_mode=WAL;PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    add(&c, 1, &message("openai-completions"), false);
    let cfg = config(&p);
    let cfg_path = d.path().join("config.json");
    std::fs::write(&cfg_path, serde_json::to_vec(&cfg).unwrap()).unwrap();
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let target = d.path().join(format!("a.db{suffix}"));
        let before = std::fs::read(&target).ok();
        assert_eq!(
            output::validate_destination(&target, &cfg_path, None, &cfg)
                .unwrap_err()
                .0,
            "output_inside_source"
        );
        assert_eq!(
            collect(&cfg, Some(&target)).unwrap_err().0,
            "cache_inside_source"
        );
        assert_eq!(std::fs::read(target).ok(), before);
    }
}

#[test]
fn zero_token_assistant_response_still_has_one_message() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = db(&p);
    let mut m = message("openai-completions");
    m["message"]["usage"] = json!({"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"reasoningTokens":0,"totalTokens":0});
    add(&c, 1, &m, false);
    let s = collect(&config(&p), None).unwrap();
    assert_eq!(s.totals.total().unwrap(), 0);
    assert_eq!(s.rows[0].messages, MessageCount::one());
}
