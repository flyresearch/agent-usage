use std::io::Write;
mod support;
use agent_usage::{
    locking::{CollectionLocks, LOCK_FILE_NAME},
    Config, Source,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
};

// Keep outer fixtures serial: this harness spawns subprocesses while advisory
// file locks are held. Every contention scenario below creates its own explicit
// competing process, rather than relying on unrelated test scheduling.
static PROCESS_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Fixture {
    root: tempfile::TempDir,
    config: Config,
    config_path: PathBuf,
    cache: PathBuf,
    output: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for dir in ["source", "config", "state"] {
            fs::create_dir(root.path().join(dir)).unwrap();
        }
        let line = json!({"type":"event_msg","timestamp":"2026-01-01T00:00:00Z","payload":{"type":"token_count","model":"test-model","info":{"total_token_usage":{"input_tokens":10,"output_tokens":2}}}});
        fs::write(root.path().join("source/log.jsonl"), format!("{line}\n")).unwrap();
        let config = Config {
            schema_version: 1,
            host_id: "fixture".into(),
            model_aliases: std::collections::BTreeMap::new(),
            sources: vec![Source {
                id: "codex".into(),
                adapter: "codex-jsonl".into(),
                paths: vec![root.path().join("source")],
            }],
        };
        let config_path = root.path().join("config/config.json");
        fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let cache = root.path().join("state/cache.json");
        let output = root.path().join("state/snapshot.json");
        Self {
            root,
            config,
            config_path,
            cache,
            output,
        }
    }
    fn lock(&self) -> CollectionLocks {
        CollectionLocks::acquire(
            &self.config_path,
            Some(&self.cache),
            Some(&self.output),
            &self.config,
        )
        .unwrap()
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(support::binary());
        cmd.arg("collect")
            .arg("--config")
            .arg(&self.config_path)
            .arg("--cache")
            .arg(&self.cache)
            .arg("--output")
            .arg(&self.output)
            .arg("--summary");
        cmd
    }
    fn run(&self) -> Output {
        self.command().output().unwrap()
    }
}
fn error(result: &Output) -> String {
    assert!(!result.status.success());
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["complete"], false);
    assert!(value.get("totals").is_none());
    value["error"].as_str().unwrap().to_owned()
}
#[test]
fn overlapping_cli_is_rejected_and_preserves_last_good() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    let f = Fixture::new();
    assert!(f.run().status.success());
    let saved = fs::read(&f.output).unwrap();
    let cache = fs::read(&f.cache).unwrap();
    let locks = f.lock();
    let busy = f.run();
    assert_eq!(error(&busy), "collection_busy");
    assert_eq!(fs::read(&f.output).unwrap(), saved);
    assert_eq!(fs::read(&f.cache).unwrap(), cache);
    let text = String::from_utf8(busy.stderr).unwrap();
    assert_eq!(text.trim(), "agent-usage: collection_busy");
    drop(locks);
    assert!(f.run().status.success());
}
#[test]
fn report_remains_available_while_collection_is_locked() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    let f = Fixture::new();
    assert!(f.run().status.success());
    let _locks = f.lock();
    let report = Command::new(support::binary())
        .arg("report")
        .arg("--snapshot")
        .arg(&f.output)
        .output()
        .unwrap();
    assert!(report.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&report.stdout).unwrap()["known_total_tokens"],
        12
    );
}
#[test]
fn different_configs_sharing_state_cannot_publish_concurrently() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    let f = Fixture::new();
    let second = f.root.path().join("other-config");
    fs::create_dir(&second).unwrap();
    let second_config = second.join("other.json");
    fs::write(&second_config, serde_json::to_vec(&f.config).unwrap()).unwrap();
    let first = f.lock();
    assert_eq!(
        CollectionLocks::acquire(&second_config, Some(&f.cache), None, &f.config)
            .unwrap_err()
            .0,
        "collection_busy"
    );
    assert_eq!(
        CollectionLocks::acquire(&second_config, None, Some(&f.output), &f.config)
            .unwrap_err()
            .0,
        "collection_busy"
    );
    drop(first);
    let retry = CollectionLocks::acquire(&second_config, Some(&f.cache), None, &f.config);
    assert!(retry.is_ok(), "retry failed: {:?}", retry.err());
}
#[test]
fn same_config_locks_even_without_cache_or_output() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    let f = Fixture::new();
    let _guard = CollectionLocks::acquire(&f.config_path, None, None, &f.config).unwrap();
    let out = Command::new(support::binary())
        .arg("collect")
        .arg("--config")
        .arg(&f.config_path)
        .output()
        .unwrap();
    assert_eq!(error(&out), "collection_busy");
}
#[test]
fn disjoint_directories_are_independent() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    let a = Fixture::new();
    let b = Fixture::new();
    let _guard = a.lock();
    assert!(b.run().status.success());
}
#[test]
fn partial_lock_acquisition_is_released_on_failure() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    let f = Fixture::new();
    // config sorts before state: contention on state must release config again.
    let state_config = f.root.path().join("state/other.json");
    fs::write(&state_config, serde_json::to_vec(&f.config).unwrap()).unwrap();
    let held = CollectionLocks::acquire(&state_config, None, None, &f.config).unwrap();
    assert_eq!(error(&f.run()), "collection_busy");
    assert!(CollectionLocks::acquire(&f.config_path, None, None, &f.config).is_ok());
    drop(held);
    assert!(f.run().status.success());
}
#[test]
fn persistent_marker_is_empty_and_reused() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    let f = Fixture::new();
    let marker = f.config_path.parent().unwrap().join(LOCK_FILE_NAME);
    {
        let _guard = f.lock();
    }
    assert_eq!(fs::read(&marker).unwrap(), b"");
    let before = fs::metadata(&marker).unwrap();
    {
        let _guard = f.lock();
    }
    let after = fs::metadata(&marker).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        assert_eq!(before.ino(), after.ino());
        assert_eq!(after.permissions().mode() & 0o777, 0o600);
    }
    #[cfg(not(unix))]
    {
        assert_eq!(before.created().unwrap(), after.created().unwrap());
    }
}
#[test]
fn reserved_marker_cannot_be_used_as_a_state_target() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    let f = Fixture::new();
    let reserved = f.cache.parent().unwrap().join(LOCK_FILE_NAME);
    assert_eq!(
        CollectionLocks::acquire(&f.config_path, Some(&reserved), None, &f.config)
            .unwrap_err()
            .0,
        "lock_path_conflict"
    );
    assert!(!reserved.exists());
    assert!(!f
        .config_path
        .parent()
        .unwrap()
        .join(LOCK_FILE_NAME)
        .exists());
}
#[test]
fn lock_markers_are_never_created_in_source_directories() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    let f = Fixture::new();
    let bad = f.root.path().join("source/config.json");
    fs::write(&bad, serde_json::to_vec(&f.config).unwrap()).unwrap();
    assert_eq!(
        CollectionLocks::acquire(&bad, None, None, &f.config)
            .unwrap_err()
            .0,
        "lock_inside_source"
    );
    assert!(!f.root.path().join("source").join(LOCK_FILE_NAME).exists());
}
#[test]
fn nonempty_marker_is_not_modified() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    let f = Fixture::new();
    let marker = f.config_path.parent().unwrap().join(LOCK_FILE_NAME);
    fs::write(&marker, "PRIVATE_SENTINEL").unwrap();
    let out = f.run();
    assert_eq!(error(&out), "invalid_lock_file");
    assert_eq!(fs::read_to_string(marker).unwrap(), "PRIVATE_SENTINEL");
    assert!(!String::from_utf8_lossy(&out.stderr).contains("PRIVATE"));
}
#[cfg(unix)]
#[test]
fn symlink_and_hardlink_markers_are_rejected() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let sentinel = f.root.path().join("sentinel");
    fs::write(&sentinel, "").unwrap();
    let marker = f.config_path.parent().unwrap().join(LOCK_FILE_NAME);
    symlink(&sentinel, &marker).unwrap();
    assert_eq!(error(&f.run()), "invalid_lock_file");
    fs::remove_file(&marker).unwrap();
    fs::hard_link(&sentinel, &marker).unwrap();
    assert_eq!(error(&f.run()), "invalid_lock_file");
    assert_eq!(fs::read(sentinel).unwrap(), b"");
}
#[cfg(unix)]
#[test]
fn directory_alias_uses_the_same_lock() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    let f = Fixture::new();
    let alias = f.root.path().join("alias");
    std::os::unix::fs::symlink(f.config_path.parent().unwrap(), &alias).unwrap();
    let _guard = f.lock();
    let out = Command::new(support::binary())
        .arg("collect")
        .arg("--config")
        .arg(alias.join("config.json"))
        .output()
        .unwrap();
    assert_eq!(error(&out), "collection_busy");
}

// Real process helper: never built into the production CLI. The parent tests
// control stdin/stdout, avoiding timing-based races or production test hooks.
#[test]
#[ignore]
fn lock_holder_process() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    let Some(path) = std::env::var_os("AGENT_USAGE_TEST_LOCK_CONFIG") else {
        return;
    };
    let path = PathBuf::from(path);
    let config: Config = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let _guard = CollectionLocks::acquire(&path, None, None, &config).unwrap();
    println!("LOCK_READY");
    std::io::stdout().flush().unwrap();
    let _ = std::io::stdin().read_to_end(&mut Vec::new());
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            return;
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn killed_process_releases_os_lock_without_deleting_marker() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    let f = Fixture::new();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_holder_process", "--ignored", "--nocapture"])
        .env("AGENT_USAGE_TEST_LOCK_CONFIG", &f.config_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    let mut reader = BufReader::new(child.0.stdout.take().unwrap());
    loop {
        let mut line = String::new();
        assert_ne!(reader.read_line(&mut line).unwrap(), 0);
        if line.trim() == "LOCK_READY" {
            break;
        }
    }
    assert_eq!(error(&f.run()), "collection_busy");
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    assert!(f
        .config_path
        .parent()
        .unwrap()
        .join(LOCK_FILE_NAME)
        .exists());
    assert!(f.run().status.success());
}

#[cfg(unix)]
fn collect_with_config_edit(edit: impl FnOnce(&Fixture)) -> (Fixture, Output, Vec<u8>) {
    use std::{sync::mpsc, time::Duration};
    let f = Fixture::new();
    assert!(f.run().status.success());
    let saved = fs::read(&f.output).unwrap();
    let cached = fs::read(&f.cache).unwrap();
    fs::remove_file(&f.cache).unwrap();
    assert!(Command::new("mkfifo")
        .arg(&f.cache)
        .status()
        .unwrap()
        .success());
    let cache_path = f.cache.clone();
    let cached_for_feeder = cached.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    // A FIFO is a test-only barrier at the real cache read. It guarantees that
    // config has already been read/rechecked and the collection locks are held.
    let feeder = std::thread::spawn(move || {
        let mut writer = fs::OpenOptions::new().write(true).open(cache_path).unwrap();
        entered_tx.send(()).unwrap();
        if resume_rx.recv().is_ok() {
            writer.write_all(&cached_for_feeder).unwrap();
        }
    });
    let child = f
        .command()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("collector did not enter the cache read");
    edit(&f);
    resume_tx.send(()).unwrap();
    feeder.join().unwrap();
    let mut stdout = Vec::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut stdout)
        .unwrap();
    let mut stderr = Vec::new();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    let status = child.0.wait().unwrap();
    // An unchanged cache is deliberately not rewritten by collect. Restore the
    // test-only FIFO before exercising a normal retry.
    use std::os::unix::fs::FileTypeExt;
    if fs::symlink_metadata(&f.cache)
        .unwrap()
        .file_type()
        .is_fifo()
    {
        fs::remove_file(&f.cache).unwrap();
        fs::write(&f.cache, cached).unwrap();
    }
    (
        f,
        Output {
            status,
            stdout,
            stderr,
        },
        saved,
    )
}
#[cfg(unix)]
#[test]
fn config_replaced_or_removed_during_collection_cannot_publish() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    for case in ["replace", "remove", "malformed"] {
        let (f, result, saved) = collect_with_config_edit(|f| match case {
            "replace" => {
                let mut config = f.config.clone();
                config.host_id = "changed-host".into();
                let mut temp =
                    tempfile::NamedTempFile::new_in(f.config_path.parent().unwrap()).unwrap();
                serde_json::to_writer(temp.as_file_mut(), &config).unwrap();
                temp.persist(&f.config_path).unwrap();
            }
            "remove" => {
                fs::remove_file(&f.config_path).unwrap();
            }
            _ => {
                fs::write(&f.config_path, "PRIVATE_INVALID_JSON").unwrap();
            }
        });
        assert_eq!(error(&result), "configuration_changed");
        assert_eq!(fs::read(&f.output).unwrap(), saved);
        assert!(!String::from_utf8_lossy(&result.stderr).contains("PRIVATE"));
        fs::write(&f.config_path, serde_json::to_vec(&f.config).unwrap()).unwrap();
        assert!(f.run().status.success());
    }
}
#[cfg(unix)]
#[test]
fn formatting_only_config_edit_does_not_invalidate_publication() {
    let _serial = PROCESS_TESTS.lock().unwrap();
    let (f, result, _) = collect_with_config_edit(|f| {
        fs::write(
            &f.config_path,
            serde_json::to_string_pretty(&f.config).unwrap(),
        )
        .unwrap();
    });
    assert!(result.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["known_total_tokens"],
        12
    );
    assert!(f.run().status.success());
}
