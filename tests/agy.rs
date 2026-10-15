use agent_usage::*;
use rusqlite::{params, Connection};
use std::{collections::BTreeMap, fs, path::Path};
fn var(mut n: u64) -> Vec<u8> {
    let mut b = Vec::new();
    while n >= 128 {
        b.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    b.push(n as u8);
    b
}
fn integer(field: u64, n: u64) -> Vec<u8> {
    let mut b = var(field << 3);
    b.extend(var(n));
    b
}
fn blob(field: u64, value: &[u8]) -> Vec<u8> {
    let mut b = var(field << 3 | 2);
    b.extend(var(value.len() as u64));
    b.extend(value);
    b
}
fn timestamp(sec: u64) -> Vec<u8> {
    integer(1, sec)
}
fn instant() -> u64 {
    chrono::DateTime::parse_from_rfc3339("2026-01-01T16:00:00Z")
        .unwrap()
        .timestamp() as u64
}
fn generation(id: Option<&str>, native: Option<u64>) -> Vec<u8> {
    let mut usage = Vec::new();
    for (n, value) in [(1, 3), (2, 10), (3, 9), (5, 100), (9, 7), (10, 2)] {
        usage.extend(integer(n, value));
    }
    if let Some(id) = id {
        usage.extend(blob(11, id.as_bytes()));
    }
    let mut chat = blob(4, &usage);
    chat.extend(blob(19, b"synthetic-model"));
    if let Some(sec) = native {
        chat.extend(blob(9, &blob(4, &timestamp(sec))));
    }
    blob(1, &chat)
}
fn step(id: Option<&str>, index: Option<u64>, sec: u64) -> Vec<u8> {
    let mut b = blob(1, &timestamp(sec));
    if let Some(id) = id {
        b.extend(blob(9, &blob(11, id.as_bytes())));
    }
    if let Some(idx) = index {
        b.extend(blob(20, &integer(3, idx)));
    }
    b
}
fn database(path: &Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("CREATE TABLE gen_metadata(idx INTEGER PRIMARY KEY,data BLOB); CREATE TABLE steps(step_type INTEGER,metadata BLOB);").unwrap();
    conn
}
fn add(conn: &Connection, index: i64, data: Vec<u8>) {
    conn.execute(
        "INSERT INTO gen_metadata VALUES(?1,?2)",
        params![index, data],
    )
    .unwrap();
}
fn aliases() -> BTreeMap<String, String> {
    BTreeMap::from([("synthetic-model".into(), "test-model".into())])
}
fn config(path: &Path) -> Config {
    Config {
        schema_version: 1,
        host_id: "fixture".into(),
        model_aliases: aliases(),
        sources: vec![Source {
            id: "agy".into(),
            adapter: "agy-sqlite".into(),
            paths: vec![path.into()],
        }],
    }
}
#[test]
fn native_usage_is_mutually_exclusive_and_duplicates_do_not_add() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.db");
    let conn = database(&path);
    add(&conn, 0, generation(Some("response"), Some(instant())));
    add(&conn, 1, generation(Some("response"), Some(instant())));
    let s = collect(&config(&path), None).unwrap();
    assert_eq!(s.rows[0].messages, MessageCount::one());
    assert_eq!(
        s.totals,
        Tokens {
            input: 13,
            cache_read: 100,
            cache_write: 0,
            output: 7,
            reasoning: 2
        }
    );
    assert_eq!(s.rows[0].day.as_deref(), Some("2026-01-02"));
    assert_eq!(s.rows[0].agent, "agy");
    assert_eq!(s.rows[0].model, "test-model");
    assert_eq!(s.sqlite_version.as_deref(), Some(rusqlite::version()));
    assert_eq!(s.sources[0].coverage.usage_records, 2);
    assert_eq!(output::summary(&s).unwrap()["known_total_tokens"], 122);
}
#[test]
fn committed_wal_is_visible_and_never_served_from_metadata_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("live.db");
    let conn = database(&path);
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    add(&conn, 0, generation(Some("first"), Some(instant())));
    let before = fs::read(&path).unwrap();
    let wal = fs::read(path.with_file_name("live.db-wal")).unwrap();
    let cache = dir.path().join("cache.json");
    let first = collect(&config(&path), Some(&cache)).unwrap();
    assert_eq!(first.totals.total().unwrap(), 122);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(path.with_file_name("live.db-wal")).unwrap(), wal);
    add(&conn, 1, generation(Some("second"), Some(instant())));
    assert_eq!(fs::read(&path).unwrap(), before); // The source main DB did not change.
    let next = collect(&config(&path), Some(&cache)).unwrap();
    assert_eq!(next.totals.total().unwrap(), 244);
    assert_eq!(next.sources[0].cache_hits, 0);
}
#[test]
fn uncommitted_wal_is_not_visible() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.db");
    let conn = database(&path);
    conn.execute_batch("PRAGMA journal_mode=WAL; BEGIN IMMEDIATE;")
        .unwrap();
    add(&conn, 0, generation(Some("uncommitted"), Some(instant())));
    assert_eq!(
        collect(&config(&path), None)
            .unwrap()
            .totals
            .total()
            .unwrap(),
        0
    );
    conn.execute_batch("COMMIT").unwrap();
    assert_eq!(
        collect(&config(&path), None)
            .unwrap()
            .totals
            .total()
            .unwrap(),
        122
    );
}
#[test]
fn source_connection_is_readonly_and_missing_db_is_not_created() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.db");
    let _conn = database(&path);
    let reader = sqlite::open_readonly(&path).unwrap();
    assert!(reader
        .execute("INSERT INTO gen_metadata VALUES(1,x'')", [])
        .is_err());
    assert_eq!(
        reader
            .query_row("SELECT sqlite_version()", [], |r| r.get::<_, String>(0))
            .unwrap(),
        rusqlite::version()
    );
    let missing = dir.path().join("missing.db");
    assert!(sqlite::open_readonly(&missing).is_err());
    assert!(!missing.exists());
}
#[test]
fn step_response_and_index_joins_supply_dates() {
    for with_response in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.db");
        let conn = database(&path);
        add(&conn, 7, generation(Some("response"), None));
        let s = step(
            if with_response {
                Some("response")
            } else {
                None
            },
            if with_response { None } else { Some(7) },
            instant(),
        );
        conn.execute("INSERT INTO steps VALUES(15,?1)", [s])
            .unwrap();
        assert_eq!(
            collect(&config(&path), None).unwrap().rows[0]
                .day
                .as_deref(),
            Some("2026-01-02")
        );
    }
}
#[test]
fn conflicting_step_time_is_unallocated_not_arbitrarily_selected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.db");
    let conn = database(&path);
    add(&conn, 0, generation(Some("response"), None));
    for t in [instant(), instant() + 86400] {
        conn.execute(
            "INSERT INTO steps VALUES(15,?1)",
            [step(Some("response"), Some(0), t)],
        )
        .unwrap();
    }
    let s = collect(&config(&path), None).unwrap();
    assert!(s.rows[0].day.is_none());
    assert_eq!(s.sources[0].coverage.ambiguous_time_records, 1);
    assert_eq!(s.sources[0].coverage.unallocated_tokens, 122);
    assert!(!s.coverage_complete);
}
#[test]
fn no_time_does_not_fall_back_to_session_or_file_timestamp() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.db");
    let conn = database(&path);
    add(&conn, 0, generation(Some("response"), None));
    conn.execute_batch("CREATE TABLE trajectory_metadata_blob(data BLOB)")
        .unwrap();
    conn.execute(
        "INSERT INTO trajectory_metadata_blob VALUES(?1)",
        [blob(2, &timestamp(instant()))],
    )
    .unwrap();
    let s = collect(&config(&path), None).unwrap();
    assert!(s.rows[0].day.is_none());
    assert_eq!(s.sources[0].coverage.missing_time_records, 1);
}
#[test]
fn missing_identity_is_explicit_and_records_are_not_collapsed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.db");
    let conn = database(&path);
    for i in 0..2 {
        add(&conn, i, generation(None, Some(instant())));
    }
    let s = collect(&config(&path), None).unwrap();
    assert_eq!(s.totals.total().unwrap(), 244);
    assert_eq!(s.sources[0].coverage.missing_identity_records, 2);
    assert!(!s.coverage_complete);
}
#[test]
fn different_usage_for_same_response_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.db");
    let conn = database(&path);
    add(&conn, 0, generation(Some("response"), Some(instant())));
    add(
        &conn,
        1,
        generation(Some("response"), Some(instant() + 86400)),
    );
    assert_eq!(
        collect(&config(&path), None).unwrap_err().0,
        "conflicting_response_replay"
    );
}
#[test]
fn malformed_protobuf_and_wrong_field_types_fail() {
    for bytes in [vec![0], vec![0x0a, 0xff], vec![0xff; 11], integer(1, 3)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.db");
        let conn = database(&path);
        add(&conn, 0, bytes);
        assert!(collect(&config(&path), None).is_err());
    }
}
#[test]
fn output_total_negative_encoding_and_duplicate_fields_fail() {
    for usage in [
        integer(9, 10),
        [integer(3, 20), integer(9, 10)].concat(),
        integer(1, u64::MAX),
        [integer(1, 10), integer(1, 20)].concat(),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.db");
        let conn = database(&path);
        add(&conn, 0, blob(1, &blob(4, &usage)));
        assert!(collect(&config(&path), None).is_err());
    }
}
#[test]
fn missing_schema_and_corruption_are_not_empty_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.db");
    let _conn = Connection::open(&path).unwrap();
    assert_eq!(
        collect(&config(&path), None).unwrap_err().0,
        "unsupported_sqlite_schema"
    );
    fs::write(dir.path().join("bad.db"), b"not sqlite").unwrap();
    assert!(collect(&config(&dir.path().join("bad.db")), None).is_err());
}
#[test]
fn directory_only_reads_direct_database_children_and_explicit_multi_paths_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.db");
    let conn = database(&path);
    add(&conn, 0, generation(Some("first"), Some(instant())));
    fs::create_dir(dir.path().join("archive")).unwrap();
    fs::write(dir.path().join("archive/invalid.db"), b"not sqlite").unwrap();
    fs::write(dir.path().join("other.jsonl"), b"not JSON").unwrap();
    assert_eq!(
        collect(&config(dir.path()), None).unwrap().sources[0].files,
        1
    );
    let dir2 = tempfile::tempdir().unwrap();
    let conn2 = database(&dir2.path().join("second.sqlite"));
    add(&conn2, 0, generation(Some("second"), Some(instant())));
    let mut c = config(&path);
    c.sources[0].paths.push(dir2.path().into());
    assert_eq!(collect(&c, None).unwrap().totals.total().unwrap(), 244);
}

#[test]
fn placeholder_without_model_or_response_is_not_billable_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.db");
    let conn = database(&path);
    add(&conn, 0, blob(1, &blob(4, &integer(1, 100))));
    let s = collect(&config(&path), None).unwrap();
    assert_eq!(s.totals.total().unwrap(), 0);
    assert_eq!(s.sources[0].coverage.unclassified_usage_records, 1);
    assert_eq!(s.sources[0].status, "partial");
    assert!(!s.coverage_complete);
}
#[test]
fn duplicate_physical_databases_and_overlapping_roots_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.db");
    let _conn = database(&path);
    let mut c = config(dir.path());
    c.sources[0].paths.push(path.clone());
    assert_eq!(collect(&c, None).unwrap_err().0, "overlapping_paths");
    fs::hard_link(&path, dir.path().join("alias.db")).unwrap();
    assert_eq!(
        collect(&config(dir.path()), None).unwrap_err().0,
        "duplicate_physical_file"
    );
}
#[test]
fn invalid_source_preserves_existing_cache_and_does_not_publish_partial() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("a.db");
    let conn = database(&good);
    add(&conn, 0, generation(Some("valid"), Some(instant())));
    let cache = dir.path().join("cache.json");
    let mut c = config(&good);
    collect(&c, Some(&cache)).unwrap();
    let before = fs::read(&cache).unwrap();
    let bad = dir.path().join("bad.db");
    fs::write(&bad, "corrupt").unwrap();
    c.sources[0].paths.push(bad);
    assert!(collect(&c, Some(&cache)).is_err());
    assert_eq!(fs::read(cache).unwrap(), before);
}
