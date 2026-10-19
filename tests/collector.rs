mod support;
use agent_usage::*;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, io::Write, process::Command};
fn aliases() -> BTreeMap<String, String> {
    [("fixture-model".into(), "test-model".into())].into()
}
fn codex(i: u64, c: u64, o: u64, r: u64, time: &str) -> Value {
    json!({"type":"event_msg","timestamp":time,"payload":{"type":"token_count","model":"fixture-model","info":{"total_token_usage":{"input_tokens":i,"cached_input_tokens":c,"output_tokens":o,"reasoning_output_tokens":r,"total_tokens":i+o}}}})
}
fn claude(output: u64, time: &str) -> Value {
    json!({"type":"assistant","timestamp":time,"requestId":"fake-request","message":{"id":"fake-message","model":"fixture-model","usage":{"input_tokens":10,"cache_read_input_tokens":100,"cache_creation_input_tokens":20,"output_tokens":output}}})
}
fn parse(v: &[Value], adapter: &str) -> Result<parser::Parsed> {
    let bytes = v.iter().map(|v| format!("{v}\n")).collect::<String>();
    parser::parse(bytes.as_bytes(), adapter, &aliases(), "synthetic-file")
}
fn sum(p: &parser::Parsed) -> Tokens {
    p.records
        .iter()
        .fold(Tokens::default(), |a, r| a.checked_add(r.tokens).unwrap())
}
const T: &str = "2026-01-01T00:00:00Z";
fn config(root: &std::path::Path) -> Config {
    Config {
        schema_version: 1,
        host_id: "test-host".into(),
        sources: vec![Source {
            id: "primary".into(),
            adapter: "codex-jsonl".into(),
            paths: vec![root.to_path_buf()],
        }],
        model_aliases: aliases(),
    }
}
fn write(path: &std::path::Path, v: &[Value]) {
    fs::write(path, v.iter().map(|v| format!("{v}\n")).collect::<String>()).unwrap()
}
#[test]
fn codex_subsets_repeat_and_delta() {
    let a = codex(1000, 800, 100, 40, T);
    let b = codex(1100, 850, 120, 45, "2026-01-01T01:00:00Z");
    let p = parse(&[a.clone(), a, b], "codex-jsonl").unwrap();
    assert_eq!(
        sum(&p),
        Tokens {
            input: 250,
            cache_read: 850,
            output: 75,
            reasoning: 45,
            cache_write: 0
        }
    );
    assert_eq!(p.records.len(), 2);
}
#[test]
fn total_carry_is_not_billable_and_not_multiplied() {
    let mut a = codex(100, 0, 20, 0, T);
    a["payload"]["info"]["total_token_usage"]["total_tokens"] = json!(7120);
    let p = parse(&[a.clone(), a], "codex-jsonl").unwrap();
    assert_eq!(sum(&p).total().unwrap(), 120);
    assert_eq!(p.coverage.unknown_total_carry, 7000);
}
#[test]
fn total_only_is_coverage() {
    let a = json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":38448}}}});
    let p = parse(&[a], "codex-jsonl").unwrap();
    assert_eq!(sum(&p).total().unwrap(), 0);
    assert_eq!(p.coverage.unknown_total_carry, 38448);
}
#[test]
fn both_first_uses_last_not_carried_total() {
    let mut a = codex(1000, 0, 100, 0, T);
    a["payload"]["info"]["last_token_usage"] = json!({"input_tokens":10,"output_tokens":5});
    assert_eq!(
        sum(&parse(&[a], "codex-jsonl").unwrap()).total().unwrap(),
        15
    );
}
#[test]
fn total_regression_fails_without_last() {
    assert_eq!(
        parse(
            &[codex(100, 0, 10, 0, T), codex(50, 0, 5, 0, T)],
            "codex-jsonl"
        )
        .unwrap_err()
        .0,
        "counter_regression_without_last"
    );
}
#[test]
fn reset_with_last_is_explicit() {
    let mut a = codex(10, 0, 1, 0, "2026-01-01T01:00:00Z");
    a["payload"]["info"]["last_token_usage"] = json!({"input_tokens":10,"output_tokens":1});
    let p = parse(&[codex(1000, 0, 100, 0, T), a], "codex-jsonl").unwrap();
    assert_eq!(sum(&p).total().unwrap(), 1111);
    assert_eq!(p.coverage.counter_resets, 1);
}
#[test]
fn invalid_counts_and_subsets_fail() {
    for field in [
        "input_tokens",
        "cached_input_tokens",
        "output_tokens",
        "reasoning_output_tokens",
    ] {
        for value in [json!(-1), json!(1.5), json!("secret"), Value::Null] {
            let mut a = codex(10, 5, 10, 5, T);
            a["payload"]["info"]["total_token_usage"][field] = value;
            assert!(parse(&[a], "codex-jsonl").is_err());
        }
    }
    assert!(parse(&[codex(10, 11, 10, 0, T)], "codex-jsonl").is_err());
    assert!(parse(&[codex(10, 0, 10, 11, T)], "codex-jsonl").is_err());
}
#[test]
fn invalid_delta_subset_fails() {
    assert!(parse(
        &[
            codex(10, 0, 10, 0, T),
            codex(11, 5, 11, 0, "2026-01-01T01:00:00Z")
        ],
        "codex-jsonl"
    )
    .is_err());
}
#[test]
fn overflow_fails() {
    let mut a = claude(1, T);
    a["message"]["usage"]["input_tokens"] = json!(u64::MAX);
    assert!(parse(&[a], "claude-jsonl").is_err());
}
#[test]
fn claude_stream_and_out_of_order() {
    let p = parse(&[claude(1, T), claude(5, T), claude(2, T)], "claude-jsonl").unwrap();
    assert_eq!(sum(&p).total().unwrap(), 135);
    assert_eq!(p.records.len(), 1);
    assert_eq!(p.coverage.streaming_regressions, 1);
}
#[test]
fn claude_conflicting_update_fails() {
    let mut b = claude(5, T);
    b["message"]["usage"]["input_tokens"] = json!(9);
    assert_eq!(
        parse(&[claude(1, T), b], "claude-jsonl").unwrap_err().0,
        "conflicting_stream_update"
    );
}
#[test]
fn claude_message_fallback_and_request_distinction() {
    let mut a = claude(1, T);
    a.as_object_mut().unwrap().remove("requestId");
    let mut b = a.clone();
    b["message"]["usage"]["output_tokens"] = json!(5);
    assert_eq!(
        sum(&parse(&[a, b], "claude-jsonl").unwrap())
            .total()
            .unwrap(),
        135
    );
    let a = claude(1, T);
    let mut b = a.clone();
    b["requestId"] = json!("another");
    assert_eq!(parse(&[a, b], "claude-jsonl").unwrap().records.len(), 2);
}
#[test]
fn ttl_subsets_not_added() {
    let mut a = claude(5, T);
    a["message"]["usage"]["cache_creation"] =
        json!({"ephemeral_5m_input_tokens":10,"ephemeral_1h_input_tokens":10});
    assert_eq!(
        sum(&parse(&[a.clone()], "claude-jsonl").unwrap())
            .total()
            .unwrap(),
        135
    );
    a["message"]["usage"]["cache_creation"]["ephemeral_1h_input_tokens"] = json!(11);
    let partial = parse(&[a], "claude-jsonl").unwrap();
    assert_eq!(partial.coverage.invalid_cache_ttl_records, 1);
    assert_eq!(sum(&partial).cache_write, 0);
    assert!(partial.records[0].cache_write_uncertain);
}
#[test]
fn time_boundary_missing_and_cross_day() {
    let p = parse(
        &[
            codex(10, 0, 1, 0, "2026-01-01T15:59:59Z"),
            codex(20, 0, 2, 0, "2026-01-01T16:00:00Z"),
            codex(30, 0, 3, 0, "bad"),
        ],
        "codex-jsonl",
    )
    .unwrap();
    assert_eq!(p.records[0].day.as_deref(), Some("2026-01-01"));
    assert_eq!(p.records[1].day.as_deref(), Some("2026-01-02"));
    assert!(p.records[2].day.is_none());
    let p = parse(
        &[
            claude(1, "2026-01-01T15:59:59Z"),
            claude(5, "2026-01-01T16:00:00Z"),
        ],
        "claude-jsonl",
    )
    .unwrap();
    assert!(p.records[0].day.is_none());
}
#[test]
fn unfinished_line_deferred_then_counted() {
    let s = codex(10, 0, 1, 0, T).to_string();
    let p = parser::parse(s.as_bytes(), "codex-jsonl", &aliases(), "f").unwrap();
    assert_eq!(p.coverage.pending_files, 1);
    assert!(p.records.is_empty());
    assert_eq!(
        parse(&[codex(10, 0, 1, 0, T)], "codex-jsonl")
            .unwrap()
            .records
            .len(),
        1
    );
    assert!(parser::parse(b"{broken\n".as_slice(), "codex-jsonl", &aliases(), "f").is_err());
}
#[test]
fn fork_without_owner_is_invalid() {
    assert_eq!(
        parse(
            &[json!({"type":"session_meta","payload":{"forked_from_id":"synthetic-parent"}})],
            "codex-jsonl"
        )
        .unwrap_err()
        .0,
        "invalid_fork_metadata"
    );
}
#[test]
fn multi_directory_cache_refresh_delete_and_config_change() {
    let t = tempfile::tempdir().unwrap();
    let a = t.path().join("a");
    let b = t.path().join("b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    write(&a.join("a.jsonl"), &[codex(10, 0, 1, 0, T)]);
    write(&b.join("b.jsonl"), &[codex(20, 0, 2, 0, T)]);
    let mut cfg = config(&a);
    cfg.sources[0].paths.push(b.clone());
    let cache = t.path().join("cache.json");
    let cold = collect(&cfg, Some(&cache)).unwrap();
    assert_eq!(cold.totals.total().unwrap(), 33);
    for _ in 0..10 {
        let warm = collect(&cfg, Some(&cache)).unwrap();
        assert_eq!(warm.totals, cold.totals);
        assert_eq!(warm.sources[0].cache_hits, 2);
        assert_eq!(warm.sources[0].bytes_read, 0);
    }
    write(&a.join("a.jsonl"), &[codex(30, 0, 3, 0, T)]);
    assert_eq!(
        collect(&cfg, Some(&cache)).unwrap().totals.total().unwrap(),
        55
    );
    fs::remove_file(b.join("b.jsonl")).unwrap();
    assert_eq!(
        collect(&cfg, Some(&cache)).unwrap().totals.total().unwrap(),
        33
    );
    cfg.host_id = "other-host".into();
    let changed = collect(&cfg, Some(&cache)).unwrap();
    assert_eq!(changed.sources[0].cache_hits, 0);
    assert_ne!(
        changed.configuration_fingerprint,
        cold.configuration_fingerprint
    );
}
#[test]
fn missing_empty_zero_and_failure_differ() {
    let t = tempfile::tempdir().unwrap();
    let mut cfg = config(t.path());
    let empty = collect(&cfg, None).unwrap();
    assert_eq!(empty.sources[0].status, "empty");
    write(&t.path().join("zero.jsonl"), &[codex(0, 0, 0, 0, T)]);
    let zero = collect(&cfg, None).unwrap();
    assert_eq!(zero.sources[0].status, "ok");
    assert_eq!(zero.sources[0].coverage.usage_records, 1);
    cfg.sources.push(Source {
        id: "missing".into(),
        adapter: "codex-jsonl".into(),
        paths: vec![t.path().join("missing")],
    });
    assert_eq!(collect(&cfg, None).unwrap_err().0, "source_missing");
}
#[test]
fn overlap_and_hardlink_rejected_but_in_scope_file_alias_is_deduplicated() {
    let t = tempfile::tempdir().unwrap();
    let nested = t.path().join("sub");
    fs::create_dir(&nested).unwrap();
    let mut c = config(t.path());
    c.sources[0].paths.push(nested);
    assert_eq!(collect(&c, None).unwrap_err().0, "overlapping_paths");
    c.sources[0].paths.pop();
    write(&t.path().join("a.jsonl"), &[codex(10, 0, 1, 0, T)]);
    fs::hard_link(t.path().join("a.jsonl"), t.path().join("b.jsonl")).unwrap();
    assert_eq!(collect(&c, None).unwrap_err().0, "duplicate_physical_file");
    #[cfg(unix)]
    {
        fs::remove_file(t.path().join("b.jsonl")).unwrap();
        std::os::unix::fs::symlink(t.path().join("a.jsonl"), t.path().join("b.jsonl")).unwrap();
        let snapshot = collect(&c, None).unwrap();
        assert_eq!(snapshot.totals.total().unwrap(), 11);
        assert_eq!(snapshot.sources[0].files, 1);
        assert_eq!(snapshot.sources[0].coverage.deduplicated_symlink_files, 1);
    }
}
#[test]
fn cli_error_never_leaks_content_or_path() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("private.jsonl"), "SECRET_BODY_INVALID\n").unwrap();
    let cfg = t.path().join("config.json");
    fs::write(&cfg, serde_json::to_vec(&config(&source)).unwrap()).unwrap();
    let out = Command::new(support::binary())
        .args(["collect", "--config"])
        .arg(cfg)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(!format!("{text}{err}").contains("SECRET"));
    assert!(!format!("{text}{err}").contains("private.jsonl"));
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["complete"], false);
    assert!(v.get("totals").is_none());
}
#[test]
fn output_and_cache_are_projected() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source");
    fs::create_dir(&source).unwrap();
    let mut a = codex(10, 0, 1, 0, T);
    a["payload"]["model"] = json!("secret@example.com");
    a["payload"]["content"] = json!("PRIVATE BODY");
    write(&source.join("secret-name.jsonl"), &[a]);
    let cache = t.path().join("cache");
    let s = collect(&config(&source), Some(&cache)).unwrap();
    let text = format!(
        "{}{}",
        serde_json::to_string(&s).unwrap(),
        fs::read_to_string(cache).unwrap()
    );
    for forbidden in [
        "secret@example.com",
        "PRIVATE BODY",
        "secret-name.jsonl",
        "fake-message",
    ] {
        assert!(!text.contains(forbidden));
    }
    assert_eq!(s.rows[0].model, "unknown");
    assert!(!s.coverage_complete);
}
#[test]
fn pending_cache_updates_after_append() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("src");
    fs::create_dir(&root).unwrap();
    let file = root.join("a.jsonl");
    fs::write(&file, codex(10, 0, 1, 0, T).to_string()).unwrap();
    let c = config(&root);
    let cache = t.path().join("cache");
    assert_eq!(
        collect(&c, Some(&cache)).unwrap().sources[0]
            .coverage
            .pending_files,
        1
    );
    fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap()
        .write_all(b"\n")
        .unwrap();
    assert_eq!(
        collect(&c, Some(&cache)).unwrap().totals.total().unwrap(),
        11
    );
}
#[test]
fn two_hosts_are_independent() {
    let t = tempfile::tempdir().unwrap();
    write(&t.path().join("a.jsonl"), &[codex(10, 0, 1, 0, T)]);
    let mut c = config(t.path());
    let a = collect(&c, None).unwrap();
    c.host_id = "second-host".into();
    let b = collect(&c, None).unwrap();
    assert_eq!(a.totals.checked_add(b.totals).unwrap().total().unwrap(), 22);
}

#[test]
fn small_regression_is_ambiguous_not_a_new_call() {
    let mut b = codex(99, 0, 10, 0, "2026-01-01T01:00:00Z");
    b["payload"]["info"]["last_token_usage"] = json!({"input_tokens":1,"output_tokens":1});
    assert_eq!(
        parse(&[codex(100, 0, 10, 0, T), b], "codex-jsonl")
            .unwrap_err()
            .0,
        "ambiguous_counter_regression"
    );
}
#[test]
fn nested_fork_without_owner_is_invalid() {
    assert_eq!(parse(&[json!({"type":"session_meta","payload":{"source":{"subagent":{"thread_spawn":{"parent_thread_id":"synthetic"}}}}})],"codex-jsonl").unwrap_err().0,"invalid_fork_metadata");
}
#[test]
fn unknown_models_cannot_hide_conflicting_streams() {
    let mut a = claude(1, T);
    a["message"]["model"] = json!("private-model-a");
    let mut b = a.clone();
    b["message"]["model"] = json!("private-model-b");
    assert_eq!(
        parse(&[a, b], "claude-jsonl").unwrap_err().0,
        "conflicting_message_model"
    );
}
#[test]
fn failed_scan_does_not_replace_cache() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("source");
    fs::create_dir(&root).unwrap();
    write(&root.join("a.jsonl"), &[codex(10, 0, 1, 0, T)]);
    let c = config(&root);
    let cache = t.path().join("cache");
    collect(&c, Some(&cache)).unwrap();
    let before = fs::read(&cache).unwrap();
    fs::write(root.join("broken.jsonl"), "broken\n").unwrap();
    assert!(collect(&c, Some(&cache)).is_err());
    assert_eq!(fs::read(&cache).unwrap(), before);
}
#[test]
fn multiple_files_claude_update_is_not_double_counted() {
    let t = tempfile::tempdir().unwrap();
    write(&t.path().join("a.jsonl"), &[claude(1, T)]);
    write(&t.path().join("b.jsonl"), &[claude(5, T)]);
    let mut c = config(t.path());
    c.sources[0].adapter = "claude-jsonl".into();
    assert_eq!(collect(&c, None).unwrap().totals.total().unwrap(), 135);
}

#[test]
fn unsupported_usage_envelopes_never_become_zero() {
    for adapter in ["codex-jsonl", "claude-jsonl"] {
        for pointer in ["usage", "data", "payload", "message"] {
            let usage = json!({"input_tokens":50,"output_tokens":12});
            let mut row = json!({"type":"result"});
            row[pointer] = if pointer == "usage" {
                usage
            } else {
                json!({"usage":usage})
            };
            assert_eq!(
                parse(&[row], adapter).unwrap_err().0,
                "unsupported_headless_usage"
            );
        }
    }
    // Tool content is not an authoritative usage envelope.
    let body = json!({"type":"user","message":{"content":[{"type":"tool_result","content":{"usage":{"input_tokens":50}}}]}});
    assert!(parse(&[body], "claude-jsonl").unwrap().records.is_empty());
}

#[test]
fn unanchored_last_cannot_be_added_again_as_total() {
    let a = json!({"type":"event_msg","timestamp":T,"payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":0}}}});
    for tail in [
        codex(20, 0, 0, 0, "2026-01-01T01:00:00Z"),
        codex(10, 0, 0, 0, "2026-01-01T01:00:00Z"),
    ] {
        assert_eq!(
            parse(&[a.clone(), tail], "codex-jsonl").unwrap_err().0,
            "ambiguous_cumulative_baseline"
        );
    }
    // A later total+last supplies a reliable increment and a future anchor.
    let mut b = codex(20, 0, 0, 0, "2026-01-01T01:00:00Z");
    b["payload"]["info"]["last_token_usage"] = json!({"input_tokens":10,"output_tokens":0});
    assert_eq!(
        sum(&parse(
            &[a, b, codex(30, 0, 0, 0, "2026-01-01T02:00:00Z")],
            "codex-jsonl"
        )
        .unwrap())
        .input,
        30
    );
}

#[test]
fn first_zero_increment_still_establishes_known_baseline() {
    let mut a = codex(100, 0, 0, 0, T);
    a["payload"]["info"]["last_token_usage"] = json!({"input_tokens":0,"output_tokens":0});
    assert_eq!(
        sum(&parse(
            &[a, codex(120, 0, 0, 0, "2026-01-01T01:00:00Z")],
            "codex-jsonl"
        )
        .unwrap())
        .input,
        20
    );
    // A later zero snapshot cannot erase an established anchor.
    assert_eq!(sum(&parse(&[codex(100,0,0,0,T),json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":0,"output_tokens":0},"last_token_usage":{"input_tokens":0,"output_tokens":0}}}}),codex(120,0,0,0,"2026-01-01T02:00:00Z")],"codex-jsonl").unwrap()).input,120);
}

#[test]
fn last_total_only_is_explicitly_unsupported() {
    for total in [None, Some(json!({"input_tokens":100,"output_tokens":0}))] {
        let mut info = json!({"last_token_usage":{"total_tokens":100}});
        if let Some(t) = total {
            info["total_token_usage"] = t;
        }
        let row = json!({"type":"event_msg","payload":{"type":"token_count","info":info}});
        assert_eq!(
            parse(&[row], "codex-jsonl").unwrap_err().0,
            "unsupported_last_total_only"
        );
    }
}

#[test]
fn claude_missing_request_transition_rejected_in_both_orders() {
    let present = claude(5, T);
    let mut absent = claude(1, T);
    absent.as_object_mut().unwrap().remove("requestId");
    for rows in [[absent.clone(), present.clone()], [present, absent]] {
        assert_eq!(
            parse(&rows, "claude-jsonl").unwrap_err().0,
            "ambiguous_message_identity"
        );
    }
    let mut empty = claude(1, T);
    empty["requestId"] = json!("");
    assert_eq!(
        parse(&[empty], "claude-jsonl").unwrap_err().0,
        "invalid_message_identity"
    );
}

#[test]
fn claude_identity_transition_rejected_across_files_and_cache() {
    for first_has_request in [false, true] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("source");
        fs::create_dir(&root).unwrap();
        let mut first = claude(1, T);
        let mut second = claude(5, T);
        if first_has_request {
            second.as_object_mut().unwrap().remove("requestId");
        } else {
            first.as_object_mut().unwrap().remove("requestId");
        }
        write(&root.join("a.jsonl"), &[first]);
        let mut c = config(&root);
        c.sources[0].adapter = "claude-jsonl".into();
        let cache = t.path().join("cache");
        collect(&c, Some(&cache)).unwrap();
        let before = fs::read(&cache).unwrap();
        write(&root.join("b.jsonl"), &[second]);
        for cache_arg in [None, Some(cache.as_path())] {
            assert_eq!(
                collect(&c, cache_arg).unwrap_err().0,
                "ambiguous_message_identity"
            );
        }
        assert_eq!(fs::read(&cache).unwrap(), before);
    }
}

#[test]
fn codex_explicit_usage_model_updates_context() {
    let mut a = codex(10, 0, 0, 0, T);
    let mut b = codex(20, 0, 0, 0, "2026-01-01T01:00:00Z");
    b["payload"].as_object_mut().unwrap().remove("model");
    let context = json!({"type":"turn_context","payload":{"model":"old-model"}});
    for field in ["model", "model_name"] {
        a["payload"].as_object_mut().unwrap().remove("model");
        a["payload"]["info"][field] = json!("fixture-model");
        let p = parse(&[context.clone(), a.clone(), b.clone()], "codex-jsonl").unwrap();
        assert!(p.records.iter().all(|r| r.model == "test-model"));
        a["payload"]["info"].as_object_mut().unwrap().remove(field);
    }
}

#[test]
fn codex_zero_and_duplicate_rows_can_update_model() {
    let mut a = codex(0, 0, 0, 0, T);
    let mut b = codex(10, 0, 0, 0, "2026-01-01T01:00:00Z");
    b["payload"].as_object_mut().unwrap().remove("model");
    assert_eq!(
        parse(&[a.clone(), b], "codex-jsonl").unwrap().records[0].model,
        "test-model"
    );
    a["payload"]["model"] = json!("");
    a["payload"]["model_name"] = json!("fixture-model");
    let b = codex(10, 0, 0, 0, "2026-01-01T01:00:00Z");
    let mut duplicate = b.clone();
    duplicate["payload"]["model"] = json!("other-model");
    let mut c = codex(20, 0, 0, 0, "2026-01-01T02:00:00Z");
    c["payload"].as_object_mut().unwrap().remove("model");
    let p = parse(&[a, b, duplicate, c], "codex-jsonl").unwrap();
    assert_eq!(p.records.last().unwrap().model_key, hash(b"other-model"));
}

#[test]
fn cli_bare_cache_filename_works_cold_and_warm() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("source");
    fs::create_dir(&root).unwrap();
    write(&root.join("a.jsonl"), &[codex(10, 0, 0, 0, T)]);
    fs::write(
        t.path().join("config.json"),
        serde_json::to_vec(&config(&root)).unwrap(),
    )
    .unwrap();
    for expected_hits in [0, 1] {
        let result = Command::new(support::binary())
            .current_dir(t.path())
            .args([
                "collect",
                "--config",
                "config.json",
                "--cache",
                "cache.json",
            ])
            .output()
            .unwrap();
        assert!(result.status.success());
        let value: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(value["sources"][0]["cache_hits"], expected_hits);
        assert_eq!(value["totals"]["input"], 10);
    }
}

#[test]
fn parser_revision_invalidates_old_cache_layout() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("source");
    fs::create_dir(&root).unwrap();
    write(&root.join("a.jsonl"), &[claude(5, T)]);
    let mut c = config(&root);
    c.sources[0].adapter = "claude-jsonl".into();
    let cache = t.path().join("cache");
    collect(&c, Some(&cache)).unwrap();
    let mut v: Value = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
    v["version"] = json!("p2a-1");
    for file in v["files"].as_object_mut().unwrap().values_mut() {
        for record in file["parsed"]["records"].as_array_mut().unwrap() {
            record.as_object_mut().unwrap().remove("message_key");
            record.as_object_mut().unwrap().remove("request_present");
            record["tokens"]["input"] = json!(999);
        }
    }
    fs::write(&cache, serde_json::to_vec(&v).unwrap()).unwrap();
    let result = collect(&c, Some(&cache)).unwrap();
    assert_eq!(result.sources[0].cache_hits, 0);
    assert_eq!(result.totals.input, 10);
    assert_eq!(result.parser_version, "p2b-9");
}

#[test]
fn known_record_type_cannot_hide_usage_in_another_envelope() {
    let codex = json!({"type":"event_msg","payload":{"type":"token_count","usage":{"input_tokens":50,"output_tokens":12}}});
    let claude =
        json!({"type":"assistant","data":{"usage":{"input_tokens":50,"output_tokens":12}}});
    assert_eq!(
        parse(&[codex], "codex-jsonl").unwrap_err().0,
        "unsupported_usage_record"
    );
    assert_eq!(
        parse(&[claude], "claude-jsonl").unwrap_err().0,
        "unsupported_usage_record"
    );
}

#[test]
fn incomplete_codex_usage_record_is_not_silently_ignored() {
    let row = json!({"type":"token_usage_record","payload":{"usage":{"input_tokens":10,"output_tokens":2}}});
    assert_eq!(
        parse(&[row], "codex-jsonl").unwrap_err().0,
        "missing_response_identity"
    );
}

#[test]
fn contradictory_cache_write_is_excluded_across_streams_and_cache() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("source");
    fs::create_dir(&root).unwrap();
    let valid = claude(1, T);
    let mut bad = claude(5, T);
    bad["message"]["usage"]["cache_creation_input_tokens"] = json!(0);
    bad["message"]["usage"]["cache_creation"] = json!({"ephemeral_1h_input_tokens":696});
    let mut c = config(&root);
    c.sources[0].adapter = "claude-jsonl".into();
    let cache = t.path().join("cache");
    for (first, second) in [(valid.clone(), bad.clone()), (bad, valid)] {
        write(&root.join("a.jsonl"), &[first]);
        write(&root.join("b.jsonl"), &[second]);
        for _ in 0..2 {
            let result = collect(&c, Some(&cache)).unwrap();
            assert!(result.complete);
            assert!(!result.coverage_complete);
            assert_eq!(result.totals.cache_write, 0);
            assert_eq!(result.totals.input, 10);
            assert_eq!(result.totals.output, 5);
            assert_eq!(result.sources[0].status, "partial");
            assert_eq!(result.sources[0].coverage.invalid_cache_ttl_records, 1);
        }
    }
}

#[test]
fn unchanged_cache_is_not_replaced() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("source");
    fs::create_dir(&root).unwrap();
    write(&root.join("a.jsonl"), &[codex(10, 0, 1, 0, T)]);
    let c = config(&root);
    let cache = t.path().join("cache");
    collect(&c, Some(&cache)).unwrap();
    let before = fs::metadata(&cache).unwrap();
    let result = collect(&c, Some(&cache)).unwrap();
    let after = fs::metadata(&cache).unwrap();
    assert_eq!(result.sources[0].cache_hits, 1);
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(before.ino(), after.ino());
    }
}

#[cfg(unix)]
#[test]
fn nested_symlinks_cannot_expand_scope_or_hide_directories() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(&outside.path().join("a.jsonl"), &[codex(10, 0, 1, 0, T)]);
    let alias = root.path().join("alias.jsonl");
    for target in [
        outside.path().join("a.jsonl"),
        outside.path().to_path_buf(),
        root.path().join("missing.jsonl"),
    ] {
        symlink(target, &alias).unwrap();
        assert_eq!(
            collect(&config(root.path()), None).unwrap_err().0,
            "nested_symlink"
        );
        fs::remove_file(&alias).unwrap();
    }
    let inside = root.path().join("nested");
    fs::create_dir(&inside).unwrap();
    symlink(&inside, &alias).unwrap();
    assert_eq!(
        collect(&config(root.path()), None).unwrap_err().0,
        "nested_symlink"
    );
}

#[test]
fn terminal_claude_partition_replaces_conserved_provisional_input_in_both_orders() {
    let provisional = json!({"type":"assistant","timestamp":T,"message":{"id":"synthetic-partition","model":"fixture-model","stop_reason":null,"usage":{"input_tokens":100,"output_tokens":0}}});
    let terminal = json!({"type":"assistant","timestamp":T,"message":{"id":"synthetic-partition","model":"fixture-model","stop_reason":"end_turn","usage":{"input_tokens":10,"cache_read_input_tokens":60,"cache_creation_input_tokens":30,"output_tokens":20}}});
    for records in [
        vec![provisional.clone(), terminal.clone()],
        vec![terminal.clone(), provisional.clone()],
    ] {
        let parsed = parse(&records, "claude-jsonl").unwrap();
        assert_eq!(
            sum(&parsed),
            Tokens {
                input: 10,
                cache_read: 60,
                cache_write: 30,
                output: 20,
                reasoning: 0
            }
        );
        assert_eq!(parsed.coverage.input_reclassifications, 1);
    }
    for (key, value) in [
        ("stop_reason", json!(null)),
        (
            "usage",
            json!({"input_tokens":11,"cache_read_input_tokens":60,"cache_creation_input_tokens":30,"output_tokens":20}),
        ),
    ] {
        let mut bad = terminal.clone();
        bad["message"][key] = value;
        assert_eq!(
            parse(&[provisional.clone(), bad], "claude-jsonl")
                .unwrap_err()
                .0,
            "conflicting_stream_update"
        );
    }
}
#[test]
fn claude_partition_reconciliation_crosses_files_and_cache_without_guessing_unfinished_input() {
    let d = tempfile::tempdir().unwrap();
    let logs = d.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let cache = d.path().join("cache.json");
    let mut c = config(&logs);
    c.sources[0].adapter = "claude-jsonl".into();
    let preliminary = json!({"type":"assistant","timestamp":T,"message":{"id":"synthetic-partition","model":"fixture-model","usage":{"input_tokens":100,"output_tokens":0}}});
    let final_record = json!({"type":"assistant","timestamp":T,"message":{"id":"synthetic-partition","model":"fixture-model","stop_reason":"end_turn","usage":{"input_tokens":10,"cache_read_input_tokens":60,"cache_creation_input_tokens":30,"output_tokens":20}}});
    write(&logs.join("a.jsonl"), &[preliminary]);
    let first = collect(&c, Some(&cache)).unwrap();
    assert_eq!(first.totals.total().unwrap(), 0);
    assert_eq!(first.sources[0].coverage.unclassified_usage_records, 1);
    assert!(!first.coverage_complete);
    write(&logs.join("b.jsonl"), &[final_record]);
    for _ in 0..2 {
        let next = collect(&c, Some(&cache)).unwrap();
        assert_eq!(next.totals.total().unwrap(), 120);
        assert_eq!(next.totals.input, 10);
        assert_eq!(next.sources[0].coverage.input_reclassifications, 1);
        assert_eq!(next.sources[0].coverage.unclassified_usage_records, 0);
    }
}
#[test]
fn codex_exact_last_reset_requires_a_new_explicit_matching_task() {
    let task =
        |id: &str| json!({"type":"event_msg","payload":{"type":"task_started","turn_id":id}});
    let context =
        |id: &str| json!({"type":"turn_context","payload":{"turn_id":id,"model":"fixture-model"}});
    let a = codex(100, 60, 10, 0, T);
    let mut b = codex(60, 40, 5, 0, "2026-01-01T01:00:00Z");
    b["payload"]["info"]["last_token_usage"] =
        json!({"input_tokens":60,"cached_input_tokens":40,"output_tokens":5});
    let parsed = parse(
        &[
            task("one"),
            context("one"),
            a.clone(),
            task("two"),
            context("two"),
            b.clone(),
        ],
        "codex-jsonl",
    )
    .unwrap();
    assert_eq!(sum(&parsed).total().unwrap(), 175);
    assert_eq!(parsed.coverage.counter_resets, 1);
    for tail in [
        vec![task("one"), context("one")],
        vec![task("two"), context("mismatch")],
        vec![context("two")],
    ] {
        let mut records = vec![task("one"), context("one"), a.clone()];
        records.extend(tail);
        records.push(b.clone());
        assert_eq!(
            parse(&records, "codex-jsonl").unwrap_err().0,
            "ambiguous_counter_regression"
        );
    }
}

#[test]
fn global_cache_failure_is_not_attributed_to_last_validated_source() {
    let d = tempfile::tempdir().unwrap();
    let logs = d.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let cache = d.path().join("cache.json");
    fs::write(&cache, "broken cache").unwrap();
    let failure = collector::collect_detailed(&config(&logs), Some(&cache)).unwrap_err();
    assert_eq!(failure.error.0, "invalid_cache");
    assert!(failure.source_id.is_none());
}

#[test]
fn cached_projection_is_validated_before_publication_or_cache_replacement() {
    for (field, value) in [
        ("model", json!("undeclared-model")),
        ("day", json!("PRIVATE_BODY")),
        ("backend", json!("private@example.invalid")),
        ("time_basis", json!("PRIVATE_BODY")),
    ] {
        let d = tempfile::tempdir().unwrap();
        let logs = d.path().join("logs");
        fs::create_dir(&logs).unwrap();
        write(&logs.join("a.jsonl"), &[codex(10, 0, 1, 0, T)]);
        let cache = d.path().join("cache.json");
        let cfg = config(&logs);
        collect(&cfg, Some(&cache)).unwrap();
        let mut v: Value = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
        let file = v["files"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        file["parsed"]["records"][0][field] = value;
        let bytes = serde_json::to_vec(&v).unwrap();
        fs::write(&cache, &bytes).unwrap();
        assert!(collect(&cfg, Some(&cache)).is_err());
        assert_eq!(fs::read(cache).unwrap(), bytes);
    }
}

#[test]
fn messages_follow_identity_not_streams_files_or_cache_hits() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("source");
    fs::create_dir(&root).unwrap();
    write(&root.join("a.jsonl"), &[claude(1, T), claude(5, T)]);
    write(&root.join("b.jsonl"), &[claude(5, T)]);
    let mut c = config(&root);
    c.sources[0].adapter = "claude-jsonl".into();
    let cache = t.path().join("cache");
    for _ in 0..2 {
        let s = collect(&c, Some(&cache)).unwrap();
        assert_eq!(s.rows[0].messages, MessageCount::one());
        assert_eq!(output::summary(&s).unwrap()["messages"]["known"], 1);
    }
}
#[test]
fn legacy_usage_is_not_a_message_count_and_old_rows_stay_unknown() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("source");
    fs::create_dir(&root).unwrap();
    write(
        &root.join("a.jsonl"),
        &[codex(10, 0, 2, 0, T), codex(20, 0, 4, 0, T)],
    );
    let s = collect(&config(&root), None).unwrap();
    assert_eq!(s.rows[0].messages, MessageCount::default());
    let mut v = serde_json::to_value(&s).unwrap();
    for r in v["rows"].as_array_mut().unwrap() {
        r.as_object_mut().unwrap().remove("messages");
    }
    let old: Snapshot = serde_json::from_value(v).unwrap();
    assert!(!old.rows[0].messages.complete);
    assert!(MessageCount {
        known: u64::MAX,
        complete: true
    }
    .checked_add(MessageCount::one())
    .is_err());
}
