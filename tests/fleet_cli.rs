mod support;
use agent_usage::{fleet::*, *};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
fn setup(root: &Path, id: &str) -> (HostConfig, PathBuf) {
    let dir = root.join(id);
    fs::create_dir(&dir).unwrap();
    let logs = dir.join("logs");
    fs::create_dir(&logs).unwrap();
    let cfg = Config {
        schema_version: 1,
        host_id: id.into(),
        sources: vec![Source {
            id: "main".into(),
            adapter: "codex-jsonl".into(),
            paths: vec![logs.clone()],
        }],
        model_aliases: Default::default(),
    };
    let bytes = serde_json::to_vec(&cfg).unwrap();
    let path = dir.join("config.json");
    fs::write(&path, &bytes).unwrap();
    (
        HostConfig {
            host_id: id.into(),
            configuration_fingerprint: hash(&bytes),
            transport: Transport::Local {
                executable: support::binary().to_string_lossy().into_owned(),
                config: path.to_str().unwrap().into(),
                cache: None,
            },
        },
        logs,
    )
}
#[test]
fn cli_refresh_report_and_host_failure_preserve_last_good() {
    let d = tempfile::tempdir().unwrap();
    let (one, logs) = setup(d.path(), "one");
    let (two, _) = setup(d.path(), "two");
    let center = d.path().join("center");
    fs::create_dir(&center).unwrap();
    let cfg = FleetConfig {
        schema_version: 1,
        hosts: vec![one, two],
        timeout_seconds: 5,
        stale_after_seconds: 60,
    };
    let cfg_path = center.join("fleet.json");
    fs::write(&cfg_path, serde_json::to_vec(&cfg).unwrap()).unwrap();
    let store = center.join("fleet.db");
    let run = |command: &str| {
        let out = Command::new(support::binary())
            .args([command, "--config"])
            .arg(&cfg_path)
            .arg("--store")
            .arg(&store)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<FleetReport>(&out.stdout).unwrap()
    };
    let first = run("fleet-sync");
    assert!(first.complete);
    assert_eq!(first.hosts.len(), 2);
    assert_eq!(first.totals.unwrap().total().unwrap(), 0);
    let report = run("fleet-report");
    assert!(report.complete);
    fs::remove_dir(logs).unwrap();
    let failed = run("fleet-sync");
    assert!(!failed.complete);
    assert_eq!(failed.hosts[0].state, "stale");
    assert_eq!(failed.hosts[0].failed_source_id.as_deref(), Some("main"));
    assert_eq!(failed.hosts[1].state, "fresh");
    assert!(failed.hosts[0].included);
    assert!(failed.hosts[0].snapshot.is_some());
}
#[test]
fn report_does_not_create_missing_database() {
    let d = tempfile::tempdir().unwrap();
    let (one, _) = setup(d.path(), "one");
    let cfg = FleetConfig {
        schema_version: 1,
        hosts: vec![one],
        timeout_seconds: 5,
        stale_after_seconds: 60,
    };
    let path = d.path().join("fleet.json");
    fs::write(&path, serde_json::to_vec(&cfg).unwrap()).unwrap();
    let store = d.path().join("missing.db");
    assert!(run(&path, &store, false).is_err());
    assert!(!store.exists());
}

#[cfg(unix)]
#[test]
fn slow_batch_peer_does_not_make_an_older_result_look_new() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let (mut fast, _) = setup(d.path(), "fast");
    let (mut slow, _) = setup(d.path(), "slow");
    for (host, delay) in [(&mut fast, 0), (&mut slow, 3)] {
        let Transport::Local {
            executable, config, ..
        } = &mut host.transport
        else {
            unreachable!()
        };
        let source: Config = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        let snapshot = collect(&source, None).unwrap();
        fs::write(
            format!("{config}.snapshot"),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
        let script = d.path().join(format!("{}-collector", host.host_id));
        fs::write(
            &script,
            format!("#!/bin/sh\nsleep {delay}\ncat \"$3.snapshot\"\n"),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        *executable = script.to_string_lossy().into_owned();
    }
    let center = d.path().join("center");
    fs::create_dir(&center).unwrap();
    let cfg = FleetConfig {
        schema_version: 1,
        hosts: vec![fast, slow],
        timeout_seconds: 10,
        stale_after_seconds: 1,
    };
    let path = center.join("config.json");
    fs::write(&path, serde_json::to_vec(&cfg).unwrap()).unwrap();
    let report = run(&path, &center.join("fleet.db"), true).unwrap();
    assert_eq!(report.hosts[0].state, "stale");
    assert_eq!(report.hosts[1].state, "fresh");
    let first =
        chrono::DateTime::parse_from_rfc3339(report.hosts[0].last_success_at.as_ref().unwrap())
            .unwrap();
    let second =
        chrono::DateTime::parse_from_rfc3339(report.hosts[1].last_success_at.as_ref().unwrap())
            .unwrap();
    assert!(second.signed_duration_since(first).num_seconds() >= 2);
}
