mod support;
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(support::binary())
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}
fn setup() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source");
    fs::create_dir(&source).unwrap();
    let row = json!({"type":"event_msg","timestamp":"2026-01-01T00:00:00Z","payload":{"type":"token_count","model":"test-model","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":40,"cache_write_input_tokens":60,"output_tokens":10}}}});
    fs::write(source.join("a.jsonl"), format!("{row}\n")).unwrap();
    let config = json!({"schema_version":1,"host_id":"test-host","sources":[{"id":"codex","adapter":"codex-jsonl","paths":[source]}],"model_aliases":{"test-model":"test-model"}});
    fs::write(t.path().join("config.json"), config.to_string()).unwrap();
    t
}
#[test]
fn collect_publishes_snapshot_and_report_agrees() {
    let t = setup();
    let result = run(
        t.path(),
        &[
            "collect",
            "--config",
            "config.json",
            "--cache",
            "cache.json",
            "--output",
            "snapshot.json",
            "--summary",
        ],
    );
    assert!(result.status.success());
    assert!(result.stderr.is_empty());
    let summary: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(summary["known_total_tokens"], 110);
    assert_eq!(summary["known_tokens"]["cache_write"], 60);
    let snapshot: Value =
        serde_json::from_slice(&fs::read(t.path().join("snapshot.json")).unwrap()).unwrap();
    assert_eq!(snapshot["rows"].as_array().unwrap().len(), 1);
    let report = run(t.path(), &["report", "--snapshot", "snapshot.json"]);
    assert!(report.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&report.stdout).unwrap(),
        summary
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(t.path().join("snapshot.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
#[test]
fn failing_collection_keeps_previous_snapshot_and_cache() {
    let t = setup();
    let args = [
        "collect",
        "--config",
        "config.json",
        "--cache",
        "cache.json",
        "--output",
        "snapshot.json",
        "--summary",
    ];
    assert!(run(t.path(), &args).status.success());
    let snapshot = fs::read(t.path().join("snapshot.json")).unwrap();
    let cache = fs::read(t.path().join("cache.json")).unwrap();
    fs::write(t.path().join("source/a.jsonl"), "PRIVATE_BAD_JSON\n").unwrap();
    let failed = run(t.path(), &args);
    assert!(!failed.status.success());
    assert_eq!(fs::read(t.path().join("snapshot.json")).unwrap(), snapshot);
    assert_eq!(fs::read(t.path().join("cache.json")).unwrap(), cache);
    let error: Value = serde_json::from_slice(&failed.stdout).unwrap();
    assert_eq!(error["complete"], false);
    assert!(error.get("totals").is_none());
    assert!(!String::from_utf8_lossy(&failed.stderr).contains("PRIVATE"));
}
#[test]
fn output_cannot_overwrite_configuration_cache_or_source() {
    for path in ["config.json", "cache.json", "source/a.jsonl"] {
        let t = setup();
        let before = fs::read(t.path().join("config.json")).unwrap();
        let result = run(
            t.path(),
            &[
                "collect",
                "--config",
                "config.json",
                "--cache",
                "cache.json",
                "--output",
                path,
            ],
        );
        assert!(!result.status.success());
        assert_eq!(fs::read(t.path().join("config.json")).unwrap(), before);
    }
}
#[test]
fn report_rejects_failed_or_inconsistent_snapshot() {
    let t = setup();
    assert!(run(
        t.path(),
        &[
            "collect",
            "--config",
            "config.json",
            "--output",
            "snapshot.json"
        ]
    )
    .status
    .success());
    let original: Value =
        serde_json::from_slice(&fs::read(t.path().join("snapshot.json")).unwrap()).unwrap();
    for mutation in ["complete", "total", "model"] {
        let mut v = original.clone();
        match mutation {
            "complete" => v["complete"] = json!(false),
            "total" => v["totals"]["input"] = json!(100),
            _ => v["rows"][0]["model"] = json!("private@example.com"),
        };
        fs::write(t.path().join("snapshot.json"), v.to_string()).unwrap();
        let result = run(t.path(), &["report", "--snapshot", "snapshot.json"]);
        assert!(!result.status.success());
        assert!(!String::from_utf8_lossy(&result.stdout).contains("private@example.com"));
    }
}
#[test]
fn help_is_json_and_duplicate_options_fail() {
    let t = setup();
    let help = run(t.path(), &["--help"]);
    assert!(help.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&help.stdout).unwrap()["kind"],
        "help"
    );
    assert!(!run(
        t.path(),
        &[
            "collect",
            "--config",
            "config.json",
            "--config",
            "config.json"
        ]
    )
    .status
    .success());
}
#[cfg(unix)]
#[test]
fn output_symlink_is_not_followed() {
    let t = setup();
    fs::write(t.path().join("preserve"), "sentinel").unwrap();
    std::os::unix::fs::symlink("preserve", t.path().join("snapshot.json")).unwrap();
    assert!(!run(
        t.path(),
        &[
            "collect",
            "--config",
            "config.json",
            "--output",
            "snapshot.json"
        ]
    )
    .status
    .success());
    assert_eq!(
        fs::read_to_string(t.path().join("preserve")).unwrap(),
        "sentinel"
    );
}

#[test]
fn failed_source_id_is_reported_without_paths_or_raw_content() {
    let t = setup();
    let second = t.path().join("second");
    fs::create_dir(&second).unwrap();
    fs::write(second.join("bad.jsonl"), "PRIVATE_BODY_NOT_JSON\n").unwrap();
    let mut config: Value =
        serde_json::from_slice(&fs::read(t.path().join("config.json")).unwrap()).unwrap();
    config["sources"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"secondary","adapter":"claude-jsonl","paths":[second]}));
    fs::write(t.path().join("config.json"), config.to_string()).unwrap();
    let result = run(t.path(), &["collect", "--config", "config.json"]);
    assert!(!result.status.success());
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["source_id"], "secondary");
    assert_eq!(value["complete"], false);
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(
        !text.contains("PRIVATE_BODY") && !text.contains(&t.path().to_string_lossy().to_string())
    );
    fs::remove_file(second.join("bad.jsonl")).unwrap();
    fs::remove_dir(&second).unwrap();
    let result = run(t.path(), &["collect", "--config", "config.json"]);
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["source_id"], "secondary");
    assert_eq!(value["error"], "source_missing");
    config["sources"][1]["id"] = json!("private@example.invalid");
    fs::write(t.path().join("config.json"), config.to_string()).unwrap();
    let result = run(t.path(), &["collect", "--config", "config.json"]);
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(value["source_id"].is_null());
}

#[test]
fn configuration_identity_matches_collection_and_does_not_access_sources() {
    let t = setup();
    let identity = run(t.path(), &["config-id", "--config", "config.json"]);
    assert!(identity.status.success());
    let id: Value = serde_json::from_slice(&identity.stdout).unwrap();
    assert_eq!(id["kind"], "configuration_identity");
    assert!(!t.path().join(".agent-usage.lock").exists());
    let collected = run(t.path(), &["collect", "--config", "config.json"]);
    let snapshot: Value = serde_json::from_slice(&collected.stdout).unwrap();
    assert_eq!(
        id["configuration_fingerprint"],
        snapshot["configuration_fingerprint"]
    );
    fs::remove_dir_all(t.path().join("source")).unwrap();
    let after = run(t.path(), &["config-id", "--config", "config.json"]);
    assert!(after.status.success());
    assert_eq!(serde_json::from_slice::<Value>(&after.stdout).unwrap(), id);
    assert!(!t.path().join("source").exists());
}
