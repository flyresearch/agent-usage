use agent_usage::*;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs};
fn usage(input: u64, output: u64) -> Value {
    json!({"input_tokens":input,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":output,"reasoning_output_tokens":0,"total_tokens":input+output})
}
fn record(id: &str, input: u64, output: u64) -> Value {
    json!({"type":"token_usage_record","timestamp":"2026-01-01T00:00:00Z","payload":{"thread_id":"synthetic-thread","turn_id":"synthetic-turn","session_id":"synthetic-session","root_turn_id":"synthetic-root","response_id":id,"usage":usage(input,output),"turn_token_usage":usage(input,output),"thread_token_usage":usage(input,output)}})
}
fn context() -> Value {
    json!({"type":"turn_context","payload":{"turn_id":"synthetic-turn","model":"synthetic-model"}})
}
fn count(input: u64, output: u64) -> Value {
    json!({"type":"event_msg","timestamp":"2026-01-01T00:00:01Z","payload":{"type":"token_count","info":{"last_token_usage":usage(input,output),"total_token_usage":usage(input,output)}}})
}
fn parse(rows: &[Value]) -> Result<parser::Parsed> {
    let input = rows.iter().map(|v| format!("{v}\n")).collect::<String>();
    parser::parse(
        input.as_bytes(),
        "codex-jsonl",
        &BTreeMap::from([("synthetic-model".into(), "test-model".into())]),
        "synthetic-file",
    )
}
fn total(p: &parser::Parsed) -> u64 {
    p.records.iter().map(|r| r.tokens.total().unwrap()).sum()
}
#[test]
fn record_and_mirrors_count_once() {
    let p = parse(&[
        context(),
        record("one", 100, 20),
        count(100, 20),
        count(100, 20),
    ])
    .unwrap();
    assert_eq!(total(&p), 120);
    assert_eq!(p.coverage.response_usage_records, 1);
    assert_eq!(p.records[0].messages, MessageCount::one());
    assert_eq!(p.coverage.mirrored_usage_records, 2);
    assert_eq!(p.records[0].model, "test-model");
}
#[test]
fn response_dedup_uses_identity_not_equal_numbers() {
    let r = record("one", 10, 2);
    let p = parse(&[
        context(),
        r.clone(),
        r,
        count(10, 2),
        record("two", 10, 2),
        count(10, 2),
    ])
    .unwrap();
    assert_eq!(total(&p), 24);
    assert_eq!(p.records.len(), 2);
}
#[test]
fn contradictory_response_identity_fails() {
    assert_eq!(
        parse(&[record("one", 10, 2), record("one", 11, 2)])
            .unwrap_err()
            .0,
        "conflicting_response_replay"
    );
}
#[test]
fn compaction_usage_is_counted_context_estimate_is_not() {
    let mut estimate = count(0, 0);
    estimate["payload"]["info"]["last_token_usage"]["total_tokens"] = json!(500);
    let p = parse(&[
        context(),
        record("normal", 100, 20),
        count(100, 20),
        record("compact", 30, 5),
        json!({"type":"compacted","payload":{}}),
        estimate.clone(),
        estimate,
        record("next", 10, 2),
        count(10, 2),
    ])
    .unwrap();
    assert_eq!(total(&p), 167);
    assert_eq!(p.coverage.context_estimate_records, 2);
    assert_eq!(p.records[1].model, "unknown");
    assert_eq!(p.records[2].model, "test-model");
}
#[test]
fn context_estimate_requires_compaction_evidence() {
    let mut estimate = count(0, 0);
    estimate["payload"]["info"]["last_token_usage"]["total_tokens"] = json!(500);
    assert_eq!(
        parse(&[record("one", 10, 2), estimate],).unwrap_err().0,
        "unmatched_response_mirror"
    );
}
#[test]
fn unmatched_mirror_cannot_hide_downgraded_or_missing_records() {
    assert_eq!(
        parse(&[record("one", 10, 2), count(15, 3)]).unwrap_err().0,
        "unmatched_response_mirror"
    );
    let mut last_missing = count(10, 2);
    last_missing["payload"]["info"]
        .as_object_mut()
        .unwrap()
        .remove("last_token_usage");
    assert_eq!(
        parse(&[record("one", 10, 2), last_missing]).unwrap_err().0,
        "unmatched_response_mirror"
    );
}
#[test]
fn record_at_eof_does_not_need_mirror() {
    let p = parse(&[context(), record("one", 10, 2)]).unwrap();
    assert_eq!(total(&p), 12);
    assert_eq!(p.coverage.mirrored_usage_records, 0);
}
#[test]
fn legacy_prefix_is_retained_at_format_transition() {
    let prefix = count(50, 5);
    let p = parse(&[prefix, context(), record("one", 10, 2), count(10, 2)]).unwrap();
    assert_eq!(total(&p), 67);
}
#[test]
fn thread_totals_are_not_context_totals() {
    let mut r = record("one", 10, 2);
    r["payload"]["thread_token_usage"] = usage(1000, 20);
    let p = parse(&[r, count(10, 2)]).unwrap();
    assert_eq!(total(&p), 12);
}
#[test]
fn malformed_identity_usage_and_cumulative_are_not_zero() {
    for field in [
        "thread_id",
        "turn_id",
        "session_id",
        "root_turn_id",
        "response_id",
    ] {
        let mut r = record("one", 10, 2);
        r["payload"].as_object_mut().unwrap().remove(field);
        assert_eq!(parse(&[r]).unwrap_err().0, "missing_response_identity");
    }
    let mut r = record("one", 10, 2);
    r["payload"]["turn_token_usage"] = usage(9, 2);
    assert_eq!(parse(&[r]).unwrap_err().0, "invalid_response_cumulative");
    let mut r = record("one", 10, 2);
    r["payload"]["usage"]["total_tokens"] = json!(100);
    assert_eq!(parse(&[r]).unwrap_err().0, "unclassified_response_total");
    let mut r = record("one", 10, 2);
    r["payload"]["usage"]["input_tokens"] = json!(-1);
    assert_eq!(parse(&[r]).unwrap_err().0, "invalid_token_field");
}
#[test]
fn thread_identity_must_match_and_cannot_switch() {
    let metadata = json!({"type":"session_meta","payload":{"id":"different-thread"}});
    assert_eq!(
        parse(&[metadata, record("one", 10, 2)]).unwrap_err().0,
        "conflicting_response_thread"
    );
    let mut r = record("two", 10, 2);
    r["payload"]["thread_id"] = json!("different-thread");
    assert_eq!(
        parse(&[record("one", 10, 2), r]).unwrap_err().0,
        "conflicting_response_thread"
    );
}
#[test]
fn unknown_turn_and_missing_date_remain_unallocated_unknown() {
    let mut r = record("one", 10, 2);
    r["timestamp"] = Value::Null;
    r["payload"]["turn_id"] = json!("another-turn");
    let p = parse(&[context(), r]).unwrap();
    assert_eq!(p.records[0].day, None);
    assert_eq!(p.records[0].model, "unknown");
    assert_eq!(total(&p), 12);
}
#[test]
fn cache_write_is_an_input_subset() {
    let mut r = record("one", 100, 10);
    for field in ["usage", "turn_token_usage", "thread_token_usage"] {
        r["payload"][field]["cached_input_tokens"] = json!(40);
        r["payload"][field]["cache_write_input_tokens"] = json!(60);
    }
    let p = parse(&[r.clone()]).unwrap();
    assert_eq!(
        p.records[0].tokens,
        Tokens {
            input: 0,
            cache_read: 40,
            cache_write: 60,
            output: 10,
            reasoning: 0
        }
    );
    assert_eq!(total(&p), 110);
    r["payload"]["usage"]["cache_write_input_tokens"] = json!(61);
    assert_eq!(parse(&[r]).unwrap_err().0, "invalid_cache_subset");
}

#[test]
fn copied_record_files_dedup_and_mixed_storage_fails_even_warm() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("source");
    fs::create_dir(&root).unwrap();
    let r = record("one", 10, 2);
    let text = format!("{}\n{}\n", context(), r);
    fs::write(root.join("a.jsonl"), &text).unwrap();
    fs::write(root.join("b.jsonl"), &text).unwrap();
    let c = Config {
        schema_version: 1,
        host_id: "fixture".into(),
        model_aliases: BTreeMap::new(),
        sources: vec![Source {
            id: "codex".into(),
            adapter: "codex-jsonl".into(),
            paths: vec![root.clone()],
        }],
    };
    let cache = t.path().join("cache");
    for _ in 0..2 {
        assert_eq!(
            collect(&c, Some(&cache)).unwrap().totals.total().unwrap(),
            12
        );
    }
    let legacy = format!(
        "{}\n{}\n",
        json!({"type":"session_meta","payload":{"id":"synthetic-thread"}}),
        count(10, 2)
    );
    fs::write(root.join("legacy.jsonl"), legacy).unwrap();
    assert_eq!(
        collect(&c, Some(&cache)).unwrap_err().0,
        "mixed_codex_storage"
    );
}

fn fork_metadata() -> Value {
    json!({"type":"session_meta","payload":{"id":"synthetic-thread","forked_from_id":"synthetic-parent"}})
}
#[test]
fn fork_response_counts_only_child_identity() {
    let p = parse(&[
        fork_metadata(),
        context(),
        record("child", 10, 2),
        count(10, 2),
    ])
    .unwrap();
    assert_eq!(total(&p), 12);
    assert_eq!(p.coverage.unclassified_fork_prefix_records, 0);
    assert_eq!(
        p.codex_thread.as_deref(),
        Some(hash(b"synthetic-thread").as_str())
    );
}
#[test]
fn inherited_metadata_does_not_replace_child_owner() {
    let parent = json!({"type":"session_meta","payload":{"id":"synthetic-parent"}});
    let p = parse(&[
        fork_metadata(),
        parent,
        context(),
        record("child", 10, 2),
        count(10, 2),
    ])
    .unwrap();
    assert_eq!(total(&p), 12);
    assert_eq!(p.codex_thread, Some(hash(b"synthetic-thread")));
}
#[test]
fn fork_prefix_usage_is_unclassified_never_assigned_to_child() {
    let p = parse(&[
        fork_metadata(),
        count(1000, 200),
        context(),
        record("child", 10, 2),
        count(10, 2),
    ])
    .unwrap();
    assert_eq!(total(&p), 12);
    assert_eq!(p.coverage.unclassified_fork_prefix_records, 1);
}
#[test]
fn legacy_only_fork_fails_instead_of_successful_zero() {
    assert_eq!(
        parse(&[fork_metadata(), count(1000, 200)]).unwrap_err().0,
        "unsupported_legacy_fork"
    );
}
#[test]
fn nested_lineage_hint_and_both_consistent_hints_are_supported() {
    for direct in [true, false] {
        let mut meta = fork_metadata();
        meta["payload"]["source"] =
            json!({"subagent":{"thread_spawn":{"parent_thread_id":"synthetic-parent"}}});
        if !direct {
            meta["payload"]
                .as_object_mut()
                .unwrap()
                .remove("forked_from_id");
        }
        assert_eq!(total(&parse(&[meta, record("child", 10, 2)]).unwrap()), 12);
    }
}
#[test]
fn conflicting_fork_parents_are_rejected() {
    let mut meta = fork_metadata();
    meta["payload"]["source"] =
        json!({"subagent":{"thread_spawn":{"parent_thread_id":"different-parent"}}});
    assert_eq!(parse(&[meta]).unwrap_err().0, "conflicting_fork_metadata");
}
#[test]
fn replay_ancestor_chain_requires_declared_linkage() {
    let parent = json!({"type":"session_meta","payload":{"id":"synthetic-parent","forked_from_id":"synthetic-grandparent"}});
    let grandparent = json!({"type":"session_meta","payload":{"id":"synthetic-grandparent"}});
    let p = parse(&[
        fork_metadata(),
        parent,
        grandparent,
        context(),
        record("child", 10, 2),
    ])
    .unwrap();
    assert_eq!(total(&p), 12);
    let stranger = json!({"type":"session_meta","payload":{"id":"unrelated-thread"}});
    assert_eq!(
        parse(&[fork_metadata(), stranger]).unwrap_err().0,
        "unsupported_session_switch"
    );
}
#[test]
fn inherited_response_records_are_not_silently_counted() {
    let mut foreign = record("parent-response", 1000, 200);
    foreign["payload"]["thread_id"] = json!("synthetic-parent");
    assert_eq!(
        parse(&[fork_metadata(), foreign]).unwrap_err().0,
        "conflicting_response_thread"
    );
}
#[test]
fn metadata_replay_cannot_resume_after_child_usage() {
    let parent = json!({"type":"session_meta","payload":{"id":"synthetic-parent"}});
    assert_eq!(
        parse(&[fork_metadata(), record("child", 10, 2), parent])
            .unwrap_err()
            .0,
        "unsupported_session_switch"
    );
    let p = parse(&[
        fork_metadata(),
        record("child", 10, 2),
        fork_metadata(),
        record("child", 10, 2),
    ])
    .unwrap();
    assert_eq!(total(&p), 12);
}
#[test]
fn malformed_and_cyclic_fork_metadata_fails() {
    for parent in [json!(7), json!(""), json!("synthetic-thread")] {
        let mut meta = fork_metadata();
        meta["payload"]["forked_from_id"] = parent;
        assert_eq!(parse(&[meta]).unwrap_err().0, "invalid_fork_metadata");
    }
    let cycle = json!({"type":"session_meta","payload":{"id":"synthetic-parent","forked_from_id":"synthetic-thread"}});
    assert_eq!(
        parse(&[fork_metadata(), cycle]).unwrap_err().0,
        "invalid_fork_metadata"
    );
}
#[test]
fn late_fork_declaration_is_not_retroactive() {
    assert_eq!(
        parse(&[count(10, 2), fork_metadata()]).unwrap_err().0,
        "late_fork_metadata"
    );
}
#[test]
fn invalid_prefix_counters_cannot_be_hidden_as_unclassified() {
    let mut c = count(10, 2);
    c["payload"]["info"]["last_token_usage"]["input_tokens"] = json!(-1);
    assert_eq!(
        parse(&[fork_metadata(), c, record("child", 10, 2)])
            .unwrap_err()
            .0,
        "invalid_token_field"
    );
}
#[test]
fn parent_and_child_are_independent_and_warm_cache_keeps_coverage() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("source");
    fs::create_dir(&root).unwrap();
    let child = format!(
        "{}\n{}\n{}\n{}\n",
        fork_metadata(),
        count(1000, 200),
        context(),
        record("child", 10, 2)
    );
    fs::write(root.join("child.jsonl"), child).unwrap();
    let mut parent = record("parent", 1000, 200);
    parent["payload"]["thread_id"] = json!("synthetic-parent");
    fs::write(
        root.join("parent.jsonl"),
        format!("{}\n{parent}\n", context()),
    )
    .unwrap();
    let c = Config {
        schema_version: 1,
        host_id: "fixture".into(),
        model_aliases: BTreeMap::from([("synthetic-model".into(), "test-model".into())]),
        sources: vec![Source {
            id: "codex".into(),
            adapter: "codex-jsonl".into(),
            paths: vec![root],
        }],
    };
    let cache = t.path().join("cache");
    for hits in [0, 2] {
        let result = collect(&c, Some(&cache)).unwrap();
        assert_eq!(result.totals.total().unwrap(), 1212);
        assert!(!result.coverage_complete);
        assert_eq!(result.sources[0].coverage.unknown_model_records, 0);
        assert_eq!(
            result.sources[0].coverage.unclassified_fork_prefix_records,
            1
        );
        assert_eq!(result.sources[0].cache_hits, hits);
    }
}

#[test]
fn ancestor_cycles_and_changed_parent_links_are_rejected() {
    let parent = json!({"type":"session_meta","payload":{"id":"synthetic-parent","forked_from_id":"synthetic-grandparent"}});
    let cycle = json!({"type":"session_meta","payload":{"id":"synthetic-grandparent","forked_from_id":"synthetic-parent"}});
    assert_eq!(
        parse(&[fork_metadata(), parent.clone(), cycle])
            .unwrap_err()
            .0,
        "invalid_fork_metadata"
    );
    let mut changed = fork_metadata();
    changed["payload"]["forked_from_id"] = json!("synthetic-grandparent");
    assert_eq!(
        parse(&[fork_metadata(), parent, changed]).unwrap_err().0,
        "conflicting_fork_metadata"
    );
}

const CHILD_V7: &str = "00000000-0010-7000-8000-000000000000";
fn legacy_fork_meta() -> Value {
    json!({"type":"session_meta","payload":{"id":CHILD_V7,"forked_from_id":"parent"}})
}
fn turn_v7(ms: &str) -> Value {
    json!({"type":"turn_context","payload":{"turn_id":format!("00000000-{ms}-7000-8000-000000000001"),"model":"synthetic-model"}})
}
#[test]
fn legacy_fork_strictly_new_turn_preserves_inherited_baseline() {
    let mut own = count(10, 2);
    own["payload"]["info"]["total_token_usage"] = usage(110, 22);
    let p = parse(&[
        legacy_fork_meta(),
        turn_v7("000f"),
        count(100, 20),
        turn_v7("0011"),
        count(100, 20),
        own,
    ])
    .unwrap();
    assert_eq!(total(&p), 12);
    assert_eq!(p.coverage.unclassified_fork_prefix_records, 1);
}
#[test]
fn legacy_fork_same_millisecond_is_not_resolved_by_random_uuid_tail() {
    assert_eq!(
        parse(&[legacy_fork_meta(), turn_v7("0010"), count(10, 2)])
            .unwrap_err()
            .0,
        "unsupported_legacy_fork"
    );
    let mut c = turn_v7("0011");
    c["payload"]["turn_id"] = json!("not-a-v7-id");
    assert_eq!(
        parse(&[legacy_fork_meta(), c, count(10, 2)]).unwrap_err().0,
        "unsupported_legacy_fork"
    );
}
#[test]
fn legacy_fork_cannot_return_to_older_turn_after_own_usage() {
    assert_eq!(
        parse(&[
            legacy_fork_meta(),
            turn_v7("0011"),
            count(10, 2),
            turn_v7("000f")
        ])
        .unwrap_err()
        .0,
        "ambiguous_fork_turn"
    );
}
#[test]
fn legacy_cache_write_delta_is_not_double_counted() {
    let mut first = count(100, 10);
    let mut second = count(20, 2);
    for field in ["last_token_usage", "total_token_usage"] {
        first["payload"]["info"][field]["cache_write_input_tokens"] = json!(60);
        first["payload"]["info"][field]["cached_input_tokens"] = json!(40);
    }
    second["payload"]["info"]["last_token_usage"]["cache_write_input_tokens"] = json!(10);
    second["payload"]["info"]["total_token_usage"] = usage(120, 12);
    second["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(40);
    second["payload"]["info"]["total_token_usage"]["cache_write_input_tokens"] = json!(70);
    let p = parse(&[first, second]).unwrap();
    let tokens = p
        .records
        .iter()
        .try_fold(Tokens::default(), |a, r| a.checked_add(r.tokens))
        .unwrap();
    assert_eq!(
        tokens,
        Tokens {
            input: 10,
            cache_read: 40,
            cache_write: 70,
            output: 12,
            reasoning: 0
        }
    );
}
#[test]
fn early_unmetered_transcript_is_not_claimed_as_zero_usage() {
    let t = tempfile::tempdir().unwrap();
    let header =
        json!({"id":"early-session","timestamp":"2025-01-01T00:00:00Z","instructions":"synthetic"});
    let content = json!({"type":"message","role":"assistant","content":[]});
    fs::write(
        t.path().join("early.jsonl"),
        format!(
            "{header}\n{}\n{content}\n",
            json!({"record_type":"synthetic-marker"})
        ),
    )
    .unwrap();
    let c = Config {
        schema_version: 1,
        host_id: "fixture".into(),
        model_aliases: BTreeMap::new(),
        sources: vec![Source {
            id: "codex".into(),
            adapter: "codex-jsonl".into(),
            paths: vec![t.path().into()],
        }],
    };
    let s = collect(&c, None).unwrap();
    assert!(s.complete);
    assert!(!s.coverage_complete);
    assert_eq!(s.sources[0].status, "unmetered");
    assert_eq!(s.sources[0].coverage.unmetered_files, 1);
    let metered = json!({"type":"message","usage":{"input_tokens":10}});
    assert_eq!(
        parse(&[header, metered]).unwrap_err().0,
        "unsupported_usage_record"
    );
}
