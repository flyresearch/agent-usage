use agent_usage::*;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, io::Write};
fn aliases() -> BTreeMap<String, String> {
    [("fixture-model".into(), "test-model".into())].into()
}
fn row(id: &str) -> Value {
    json!({"type":"function_call","timestamp":1767283200000i64,
        "providerData":{"messageId":id,"traceId":"shared-trace","conversationRequestId":"shared-turn","model":"fixture-model",
        "usage":{"requests":1,"inputTokens":100,"outputTokens":30,"totalTokens":130,
        "inputTokensDetails":[{"cached_tokens":60}],"outputTokensDetails":[{"reasoning_tokens":10}]},
        "rawUsage":{"prompt_tokens":100,"completion_tokens":30,"total_tokens":130,"prompt_cache_hit_tokens":60,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"prompt_cache_write_tokens":0}},
        "message":{"usage":{"input_tokens":100,"output_tokens":30,"cache_read_input_tokens":60}},
        "arguments":"PRIVATE-FIXTURE","content":"PRIVATE-FIXTURE"})
}
fn parse(rows: &[Value]) -> Result<parser::Parsed> {
    let data = rows.iter().map(|v| format!("{v}\n")).collect::<String>();
    parser::parse(data.as_bytes(), "workbuddy-jsonl", &aliases(), "file")
}
fn config(root: &std::path::Path) -> Config {
    Config {
        schema_version: 1,
        host_id: "fixture-host".into(),
        model_aliases: aliases(),
        sources: vec![Source {
            id: "wb".into(),
            adapter: "workbuddy-jsonl".into(),
            paths: vec![root.to_owned()],
        }],
    }
}
fn write(path: &std::path::Path, rows: &[Value]) {
    fs::write(
        path,
        rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
    )
    .unwrap();
}
#[test]
fn splits_subsets_and_counts_calls_not_traces_or_mirrors() {
    let a = row("one");
    let mut b = row("two");
    b["type"] = json!("message");
    let p = parse(&[a.clone(), a, b]).unwrap();
    assert_eq!(p.records.len(), 2);
    for r in p.records {
        assert_eq!(
            r.tokens,
            Tokens {
                input: 40,
                cache_read: 60,
                output: 20,
                reasoning: 10,
                cache_write: 0
            }
        );
        assert_eq!(r.messages, MessageCount::one());
        assert_eq!(r.day.as_deref(), Some("2026-01-02"));
    }
}
#[test]
fn cold_warm_subagents_and_copied_prefix_are_deduplicated() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("projects");
    fs::create_dir_all(root.join("child/subagents")).unwrap();
    write(&root.join("parent.jsonl"), &[row("one")]);
    write(
        &root.join("child/subagents/child.jsonl"),
        &[row("one"), row("two")],
    );
    let cache = t.path().join("cache.json");
    let c = config(&root);
    let a = collect(&c, Some(&cache)).unwrap();
    let b = collect(&c, Some(&cache)).unwrap();
    assert_eq!(a.totals.total().unwrap(), 260);
    assert_eq!(a.rows[0].messages.known, 2);
    assert_eq!(a.totals, b.totals);
    assert_eq!(b.sources[0].cache_hits, 2);
    assert_eq!(a.rows[0].agent, "workbuddy");
    let serialized = serde_json::to_string(&a).unwrap();
    assert!(!serialized.contains("PRIVATE-FIXTURE"));
    assert!(!serialized.contains("shared-trace"));
    assert!(!fs::read_to_string(&cache)
        .unwrap()
        .contains("PRIVATE-FIXTURE"));
    // Conflict must still fail when the other copy came from warm cache.
    let mut bad = row("one");
    bad["providerData"]["model"] = json!("other-model");
    write(&root.join("child/subagents/child.jsonl"), &[bad]);
    assert_eq!(
        collect(&c, Some(&cache)).unwrap_err().0,
        "conflicting_replay"
    );
}
#[test]
fn incomplete_tail_is_deferred_then_counted() {
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("a.jsonl");
    fs::write(&f, row("one").to_string()).unwrap();
    let a = collect(&config(t.path()), None).unwrap();
    assert_eq!(a.totals.total().unwrap(), 0);
    assert!(!a.coverage_complete);
    assert_eq!(a.sources[0].coverage.pending_files, 1);
    fs::OpenOptions::new()
        .append(true)
        .open(&f)
        .unwrap()
        .write_all(b"\n")
        .unwrap();
    assert_eq!(
        collect(&config(t.path()), None)
            .unwrap()
            .totals
            .total()
            .unwrap(),
        130
    );
}
#[test]
fn invalid_or_unsupported_usage_fails_explicitly() {
    for (path, value, error) in [
        (
            "/providerData/usage/inputTokens",
            json!(-1),
            "invalid_token_field",
        ),
        (
            "/providerData/usage/outputTokens",
            json!(1.5),
            "invalid_token_field",
        ),
        (
            "/providerData/usage/totalTokens",
            json!(1),
            "invalid_reported_total",
        ),
        (
            "/providerData/usage/requests",
            json!(2),
            "unsupported_request_count",
        ),
        (
            "/providerData/usage/inputTokensDetails/0/cached_tokens",
            json!(101),
            "conflicting_usage_fields",
        ),
        (
            "/providerData/usage/outputTokensDetails/0/reasoning_tokens",
            json!(31),
            "invalid_reasoning_subset",
        ),
        (
            "/providerData/rawUsage/prompt_cache_write_tokens",
            json!(1),
            "unsupported_cache_write",
        ),
        (
            "/providerData/rawUsage/prompt_tokens",
            json!(99),
            "conflicting_usage_fields",
        ),
        (
            "/providerData/messageId",
            json!(""),
            "missing_message_identity",
        ),
        ("/type", json!("unsupported"), "unsupported_usage_record"),
    ] {
        let mut r = row("one");
        *r.pointer_mut(path).unwrap() = value;
        assert_eq!(parse(&[r]).unwrap_err().0, error, "{path}");
    }
    let mut r = row("one");
    r["providerData"]["usage"]["inputTokens"] = json!(u64::MAX);
    assert_eq!(parse(&[r]).unwrap_err().0, "token_overflow");
}
#[test]
fn missing_time_and_model_remain_visible() {
    let mut r = row("one");
    r.as_object_mut().unwrap().remove("timestamp");
    r["providerData"].as_object_mut().unwrap().remove("model");
    let p = parse(&[r]).unwrap();
    assert_eq!(p.coverage.missing_time_records, 1);
    assert_eq!(p.coverage.unknown_model_records, 1);
    assert_eq!(p.coverage.unsupported_fields, 0);
    assert_eq!(p.records[0].day, None);
}
#[test]
fn zero_token_response_counts_and_nonusage_does_not() {
    let mut r = row("zero");
    r["providerData"]["usage"] = json!({"requests":1,"inputTokens":0,"outputTokens":0,"totalTokens":0,"inputTokensDetails":[],"outputTokensDetails":[]});
    r["providerData"]["rawUsage"] = json!({"cache_creation_input_tokens":0});
    r.as_object_mut().unwrap().remove("message");
    let p = parse(&[r, json!({"type":"reasoning","content":"PRIVATE-FIXTURE"})]).unwrap();
    assert_eq!(p.records.len(), 1);
    assert_eq!(p.records[0].messages.known, 1);
    assert_eq!(p.records[0].tokens.total().unwrap(), 0);
}
#[test]
fn mirror_without_authority_and_malformed_line_fail() {
    let mut r = row("one");
    r["providerData"].as_object_mut().unwrap().remove("usage");
    assert_eq!(parse(&[r]).unwrap_err().0, "unsupported_usage_record");
    assert_eq!(
        parser::parse(&b"{broken}\n"[..], "workbuddy-jsonl", &aliases(), "file")
            .unwrap_err()
            .0,
        "invalid_jsonl"
    );
}
#[test]
fn duplicate_identity_conflict_is_not_another_call() {
    let a = row("one");
    let mut b = a.clone();
    b["providerData"]["model"] = json!("other");
    assert_eq!(parse(&[a, b]).unwrap_err().0, "conflicting_replay");
}

#[test]
fn missing_detail_and_trace_file_are_not_zero_usage() {
    let mut r = row("one");
    r["providerData"]["usage"]
        .as_object_mut()
        .unwrap()
        .remove("inputTokensDetails");
    assert_eq!(parse(&[r]).unwrap_err().0, "missing_token_field");
    assert_eq!(
        parse(&[json!({"type":"trace_end","trace":{"totalTokens":0}})])
            .unwrap_err()
            .0,
        "unsupported_record_shape"
    );
}

#[test]
fn cli_report_and_failed_refresh_preserve_last_good() {
    use std::process::Command;
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("projects");
    fs::create_dir(&root).unwrap();
    let f = root.join("session.jsonl");
    write(&f, &[row("one")]);
    let c = t.path().join("config.json");
    let out = t.path().join("snapshot.json");
    let cache = t.path().join("cache.json");
    fs::write(&c, serde_json::to_vec(&config(&root)).unwrap()).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_agent-usage"))
            .args(["collect", "--config"])
            .arg(&c)
            .arg("--cache")
            .arg(&cache)
            .arg("--output")
            .arg(&out)
            .output()
            .unwrap()
    };
    assert!(run().status.success());
    let saved = fs::read(&out).unwrap();
    let saved_cache = fs::read(&cache).unwrap();
    let report = Command::new(env!("CARGO_BIN_EXE_agent-usage"))
        .args(["report", "--snapshot"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(report.status.success());
    let json: Value = serde_json::from_slice(&report.stdout).unwrap();
    assert!(json["by_agent"].get("workbuddy").is_some());
    let mut bad = row("one");
    bad["providerData"]["rawUsage"]["prompt_cache_write_tokens"] = json!(10);
    write(&f, &[bad]);
    let failure = run();
    assert!(!failure.status.success());
    assert!(!String::from_utf8_lossy(&failure.stderr).contains("PRIVATE-FIXTURE"));
    assert_eq!(fs::read(&out).unwrap(), saved);
    assert_eq!(fs::read(&cache).unwrap(), saved_cache);
}

fn unmetered(id: Option<&str>, kind: &str) -> Value {
    let mut v = json!({"type":kind,"role":"assistant","status":"completed","providerData":{}});
    if let Some(id) = id {
        v["providerData"]["messageId"] = json!(id);
    }
    v
}

#[test]
fn unmetered_fragments_reconcile_with_terminal_usage_in_either_order() {
    for reverse in [false, true] {
        let mut rows = vec![
            unmetered(Some("one"), "message"),
            unmetered(Some("one"), "function_call"),
            row("one"),
        ];
        if reverse {
            rows.reverse();
        }
        let p = parse(&rows).unwrap();
        assert_eq!(p.records.len(), 1);
        assert_eq!(p.coverage.unclassified_usage_records, 0);
        assert_eq!(p.coverage.unmetered_files, 0);
    }
    let p = parse(&[
        row("known"),
        unmetered(Some("missing"), "message"),
        unmetered(Some("missing"), "function_call"),
        unmetered(None, "message"),
    ])
    .unwrap();
    assert_eq!(p.coverage.unclassified_usage_records, 2);
    assert_eq!(p.records.len(), 1);
}

#[test]
fn nonassistant_records_do_not_create_unmetered_gaps() {
    let p = parse(&[
        json!({"type":"message","role":"user"}),
        json!({"type":"message","role":"system"}),
        json!({"type":"function_call_result"}),
    ])
    .unwrap();
    assert!(p.records.is_empty());
    assert_eq!(p.coverage.unclassified_usage_records, 0);
    assert_eq!(p.coverage.unmetered_files, 0);
}

#[test]
fn cli_unmetered_and_mixed_sources_keep_coverage_gaps_on_warm_reads() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("projects");
    fs::create_dir(&root).unwrap();
    let config_path = t.path().join("config.json");
    let out = t.path().join("snapshot.json");
    let cache = t.path().join("cache.json");
    fs::write(&config_path, serde_json::to_vec(&config(&root)).unwrap()).unwrap();
    for mixed in [false, true] {
        let mut rows = vec![unmetered(Some("missing"), "message")];
        if mixed {
            rows.push(row("known"));
        }
        write(&root.join("session.jsonl"), &rows);
        for warm in [false, true] {
            let run = std::process::Command::new(env!("CARGO_BIN_EXE_agent-usage"))
                .arg("collect")
                .arg("--config")
                .arg(&config_path)
                .arg("--output")
                .arg(&out)
                .arg("--cache")
                .arg(&cache)
                .output()
                .unwrap();
            assert!(run.status.success());
            let s: Snapshot = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
            assert!(!s.coverage_complete);
            assert_eq!(s.sources[0].coverage.unclassified_usage_records, 1);
            assert_eq!(
                s.sources[0].status,
                if mixed { "partial" } else { "unmetered" }
            );
            assert_eq!(s.totals.total().unwrap(), if mixed { 130 } else { 0 });
            if warm {
                assert_eq!(s.sources[0].cache_hits, 1);
            }
        }
    }
}

#[test]
fn missing_write_evidence_rejects_refresh_and_preserves_snapshot_and_cache() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("projects");
    fs::create_dir(&root).unwrap();
    let f = root.join("session.jsonl");
    write(&f, &[row("one")]);
    let c = t.path().join("config.json");
    let out = t.path().join("snapshot.json");
    let cache = t.path().join("cache.json");
    fs::write(&c, serde_json::to_vec(&config(&root)).unwrap()).unwrap();
    let run = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_agent-usage"))
            .arg("collect")
            .arg("--config")
            .arg(&c)
            .arg("--output")
            .arg(&out)
            .arg("--cache")
            .arg(&cache)
            .output()
            .unwrap()
    };
    assert!(run().status.success());
    let saved = fs::read(&out).unwrap();
    let saved_cache = fs::read(&cache).unwrap();
    for raw in [None, Some(Value::Null), Some(json!({"prompt_tokens":100}))] {
        let mut bad = row("one");
        if let Some(raw) = raw {
            bad["providerData"]["rawUsage"] = raw;
        } else {
            bad["providerData"]
                .as_object_mut()
                .unwrap()
                .remove("rawUsage");
        }
        assert_eq!(
            parse(&[bad.clone()]).unwrap_err().0,
            "unsupported_cache_write"
        );
        write(&f, &[bad]);
        assert!(!run().status.success());
        assert_eq!(fs::read(&out).unwrap(), saved);
        assert_eq!(fs::read(&cache).unwrap(), saved_cache);
    }
}
