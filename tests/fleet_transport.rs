#[cfg(unix)]
use std::time::Instant;
mod support;
use agent_usage::{
    fleet::{transport::*, *},
    *,
};
use std::{fs, path::Path, time::Duration};
fn host(executable: &Path, config: &Path) -> HostConfig {
    HostConfig {
        host_id: "fixture".into(),
        configuration_fingerprint: hash(b"fixture"),
        transport: Transport::Local {
            executable: executable.to_string_lossy().into(),
            config: config.to_string_lossy().into(),
            cache: None,
        },
    }
}
#[cfg(unix)]
fn script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
#[test]
fn actual_collector_invocation_returns_valid_snapshot_and_checks_identity() {
    let d = tempfile::tempdir().unwrap();
    let logs = d.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let cfg = Config {
        schema_version: 1,
        host_id: "fixture".into(),
        sources: vec![Source {
            id: "test".into(),
            adapter: "codex-jsonl".into(),
            paths: vec![logs],
        }],
        model_aliases: Default::default(),
    };
    let path = d.path().join("config.json");
    let bytes = serde_json::to_vec(&cfg).unwrap();
    fs::write(&path, &bytes).unwrap();
    let mut h = host(&support::binary(), &path);
    h.configuration_fingerprint = hash(&bytes);
    let s = collect_host(&h, Duration::from_secs(10)).unwrap();
    assert_eq!(s.totals.total().unwrap(), 0);
    h.host_id = "wrong".into();
    assert_eq!(
        collect_host(&h, Duration::from_secs(10)).unwrap_err().0,
        "fleet_snapshot_identity_mismatch"
    );
}
#[cfg(unix)]
#[test]
fn timeout_and_broken_protocol_are_bounded() {
    let d = tempfile::tempdir().unwrap();
    let exe = d.path().join("collector");
    let cfg = d.path().join("config");
    script(&exe, "exec sleep 10");
    let h = host(&exe, &cfg);
    let started = Instant::now();
    assert_eq!(
        collect_host(&h, Duration::from_millis(80)).unwrap_err().0,
        "collection_timeout"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    script(&exe, "printf '{broken'; exit 0");
    assert_eq!(
        collect_host(&h, Duration::from_secs(1)).unwrap_err().0,
        "invalid_snapshot"
    );
    script(&exe, "printf 'private diagnostic' >&2; exit 1");
    assert_eq!(
        collect_host(&h, Duration::from_secs(1)).unwrap_err().0,
        "collection_failed"
    );
}
#[cfg(unix)]
#[test]
fn posix_remote_paths_are_literal_even_with_shell_metacharacters() {
    let d = tempfile::tempdir().unwrap();
    let exe = d.path().join("collector ' $() ;");
    script(&exe, "printf '%s\\n' \"$@\"");
    let cfg = "/synthetic/$(touch nope); ' \" config";
    let cache = "/cache/space ' and $HOME";
    let cmd = remote_command(exe.to_str().unwrap(), cfg, Some(cache), RemoteShell::Posix);
    let out = std::process::Command::new("/bin/sh")
        .args(["-c", &cmd])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("collect\n--config\n{cfg}\n--cache\n{cache}\n")
    );
}
#[test]
fn powershell_outer_command_contains_only_base64_payload() {
    let text = remote_command(
        "C:\\some path\\collector.exe",
        "C:\\config' & $(bad).json",
        None,
        RemoteShell::Powershell,
    );
    let encoded = text
        .strip_prefix("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ")
        .unwrap();
    assert!(encoded
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b)));
    assert_eq!(encoded.len() % 4, 0);
}
#[test]
fn ssh_destination_cannot_supply_options_or_commands() {
    for destination in ["-oProxyCommand=bad", "host;bad", "a b", "user@host\nother"] {
        let mut h = host(Path::new("/collector"), Path::new("/config"));
        h.transport = Transport::Ssh {
            destination: destination.into(),
            executable: "/collector".into(),
            config: "/config".into(),
            cache: None,
            shell: RemoteShell::Posix,
        };
        assert_eq!(
            collect_host(&h, Duration::from_secs(1)).unwrap_err().0,
            "invalid_ssh_destination"
        );
    }
}
