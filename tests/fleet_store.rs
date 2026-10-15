use agent_usage::{
    fleet::{store::Store, *},
    *,
};
use chrono::{Duration, TimeZone, Utc};
fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}
fn host(id: &str) -> HostConfig {
    HostConfig {
        host_id: id.into(),
        configuration_fingerprint: hash(id.as_bytes()),
        transport: Transport::Local {
            executable: std::env::temp_dir()
                .join("synthetic-collector")
                .to_string_lossy()
                .into_owned(),
            config: std::env::temp_dir()
                .join("synthetic-config")
                .to_string_lossy()
                .into_owned(),
            cache: None,
        },
    }
}
fn config(ids: &[&str]) -> FleetConfig {
    FleetConfig {
        schema_version: 1,
        hosts: ids.iter().map(|id| host(id)).collect(),
        timeout_seconds: 30,
        stale_after_seconds: 60,
    }
}
fn snapshot(id: &str, n: u64) -> Snapshot {
    serde_json::from_value(serde_json::json!({"schema_version":1,"collector_version":"0.1.0","parser_version":PARSER_VERSION,"host_id":id,"configuration_fingerprint":hash(id.as_bytes()),"generated_at":now().to_rfc3339(),"timezone":"Asia/Singapore","complete":true,"coverage_complete":true,"sources":[{"source_id":"main","status":"ok","files":1,"cache_hits":0,"bytes_read":0,"coverage":{}}],"rows":[{"source_id":"main","agent":"codex","model":"synthetic","day":"2026-01-01","time_basis":"event_timestamp","tokens":{"input":n,"output":0,"cache_read":0,"cache_write":0,"reasoning":0}}],"totals":{"input":n,"output":0,"cache_read":0,"cache_write":0,"reasoning":0},"cost":null})).unwrap()
}
#[test]
fn repeated_import_replaces_and_hosts_add_without_global_dedup() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("fleet.db")).unwrap();
    let cfg = config(&["one", "two"]);
    let g = s.activate(&cfg).unwrap();
    for id in ["one", "two"] {
        let a = s.begin(id, g, now()).unwrap();
        for _ in 0..10 {
            s.accept(&a, &snapshot(id, 10), now()).unwrap();
        }
    }
    let report = s.report(&cfg, now()).unwrap();
    assert_eq!(report.totals.unwrap().input, 20);
    assert!(report.complete);
    let a = s.begin("one", g, now()).unwrap();
    s.accept(&a, &snapshot("one", 3), now()).unwrap();
    assert_eq!(s.report(&cfg, now()).unwrap().totals.unwrap().input, 13);
}
#[test]
fn first_failure_is_missing_and_later_failure_keeps_last_good_after_restart() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("fleet.db");
    let cfg = config(&["one"]);
    let mut s = Store::open(&path).unwrap();
    let g = s.activate(&cfg).unwrap();
    let a = s.begin("one", g, now()).unwrap();
    s.fail(&a, "collection_timeout").unwrap();
    let r = s.report(&cfg, now()).unwrap();
    assert!(r.totals.is_none());
    assert_eq!(r.hosts[0].state, "never_collected");
    assert_eq!(r.hosts[0].error.as_deref(), Some("collection_timeout"));
    let a = s.begin("one", g, now()).unwrap();
    s.accept(&a, &snapshot("one", 10), now()).unwrap();
    let a = s.begin("one", g, now()).unwrap();
    s.fail(&a, "collection_failed").unwrap();
    drop(s);
    let mut s = Store::open(&path).unwrap();
    let r = s.report(&cfg, now()).unwrap();
    assert_eq!(r.totals.unwrap().input, 10);
    assert_eq!(r.hosts[0].state, "stale");
    assert!(!r.complete);
}
#[test]
fn config_aba_and_late_success_or_failure_cannot_replace_new_result() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("fleet.db")).unwrap();
    let a = config(&["one"]);
    let ga = s.activate(&a).unwrap();
    let old = s.begin("one", ga, now()).unwrap();
    s.accept(&old, &snapshot("one", 10), now()).unwrap();
    let mut b = a.clone();
    b.timeout_seconds = 31;
    let gb = s.activate(&b).unwrap();
    assert!(gb > ga);
    assert!(s.report(&b, now()).unwrap().totals.is_none());
    assert_eq!(
        s.report(&b, now()).unwrap().hosts[0].state,
        "configuration_changed"
    );
    let ga2 = s.activate(&a).unwrap();
    assert!(ga2 > gb);
    assert_eq!(
        s.accept(&old, &snapshot("one", 99), now()).unwrap_err().0,
        "fleet_stale_result"
    );
    assert!(s.fail(&old, "collection_failed").is_err());
    let first = s.begin("one", ga2, now()).unwrap();
    let second = s.begin("one", ga2, now()).unwrap();
    s.accept(&second, &snapshot("one", 20), now()).unwrap();
    assert!(s.accept(&first, &snapshot("one", 99), now()).is_err());
    assert!(s.fail(&first, "collection_timeout").is_err());
    assert_eq!(s.report(&a, now()).unwrap().totals.unwrap().input, 20);
}
#[test]
fn identity_invalid_payload_and_overflow_do_not_destroy_last_good() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("fleet.db")).unwrap();
    let cfg = config(&["one"]);
    let g = s.activate(&cfg).unwrap();
    let a = s.begin("one", g, now()).unwrap();
    s.accept(&a, &snapshot("one", 10), now()).unwrap();
    assert!(s.accept(&a, &snapshot("other", 100), now()).is_err());
    for field in ["incomplete", "fingerprint", "total", "source", "cost"] {
        let mut bad = snapshot("one", 100);
        match field {
            "incomplete" => bad.complete = false,
            "fingerprint" => bad.configuration_fingerprint = hash(b"changed"),
            "total" => bad.totals.input = 101,
            "source" => bad.sources.push(bad.sources[0].clone()),
            _ => bad.cost = Some("not-a-cost".into()),
        };
        assert!(s.accept(&a, &bad, now()).is_err());
    }
    assert_eq!(s.report(&cfg, now()).unwrap().totals.unwrap().input, 10);
}
#[test]
fn stale_age_clock_rollback_and_interrupted_attempt_are_explicit() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("fleet.db")).unwrap();
    let cfg = config(&["one"]);
    let g = s.activate(&cfg).unwrap();
    let a = s.begin("one", g, now()).unwrap();
    s.accept(&a, &snapshot("one", 10), now()).unwrap();
    for t in [now() - Duration::seconds(1), now() + Duration::seconds(61)] {
        assert_eq!(s.report(&cfg, t).unwrap().hosts[0].state, "stale");
    }
    s.begin("one", g, now()).unwrap();
    let r = s.report(&cfg, now()).unwrap();
    assert_eq!(r.hosts[0].error.as_deref(), Some("collection_interrupted"));
    assert_eq!(r.totals.unwrap().input, 10);
}
#[test]
fn removed_hosts_are_excluded_and_report_requires_current_configuration() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("fleet.db")).unwrap();
    let old = config(&["one", "two"]);
    let g = s.activate(&old).unwrap();
    let a = s.begin("two", g, now()).unwrap();
    s.accept(&a, &snapshot("two", 10), now()).unwrap();
    let new = config(&["one"]);
    s.activate(&new).unwrap();
    assert!(s.report(&old, now()).is_err());
    let r = s.report(&new, now()).unwrap();
    assert_eq!(r.hosts.len(), 1);
    assert!(r.totals.is_none());
    assert!(s.accept(&a, &snapshot("two", 100), now()).is_err());
}
#[test]
fn unrelated_source_database_is_not_modified() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("source.db");
    let c = rusqlite::Connection::open(&p).unwrap();
    c.execute_batch("CREATE TABLE usage(tokens INTEGER);INSERT INTO usage VALUES(42);")
        .unwrap();
    drop(c);
    let before = std::fs::read(&p).unwrap();
    assert!(Store::open(&p).is_err());
    assert_eq!(std::fs::read(p).unwrap(), before);
}

#[test]
fn schema_one_readonly_and_upgrade_keep_last_good_and_scope_failure_details() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("fleet.db");
    let cfg = config(&["one"]);
    let mut store = Store::open(&path).unwrap();
    let g = store.activate(&cfg).unwrap();
    let a = store.begin("one", g, now()).unwrap();
    store.accept(&a, &snapshot("one", 10), now()).unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("ALTER TABLE fleet_hosts DROP COLUMN failed_source;PRAGMA user_version=1;")
        .unwrap();
    drop(db);
    let before = std::fs::read(&path).unwrap();
    let mut readonly = Store::read_only(&path).unwrap();
    let r = readonly.report(&cfg, now()).unwrap();
    assert_eq!(r.totals.unwrap().input, 10);
    assert!(r.hosts[0].failed_source_id.is_none());
    drop(readonly);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut store = Store::open(&path).unwrap();
    let a = store.begin("one", g, now()).unwrap();
    store
        .fail_with_source(&a, "source_missing", Some("secondary"))
        .unwrap();
    let r = store.report(&cfg, now()).unwrap();
    assert_eq!(r.totals.unwrap().input, 10);
    assert_eq!(r.hosts[0].failed_source_id.as_deref(), Some("secondary"));
    assert!(store
        .fail_with_source(&a, "source_missing", Some("private@example.invalid"))
        .is_err());
    store.accept(&a, &snapshot("one", 20), now()).unwrap();
    assert!(store.report(&cfg, now()).unwrap().hosts[0]
        .failed_source_id
        .is_none());
}
