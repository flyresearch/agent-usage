use agent_usage::*;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs};

fn fixture(name: &str) -> String {
    format!(
        "{}\n",
        json!({"type":"event_msg","timestamp":"2026-01-01T00:00:00Z","payload":{"type":"token_count","model":name,"info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":40,"output_tokens":10}}}})
    )
}
fn cfg(a: &std::path::Path, b: &std::path::Path) -> Config {
    Config {
        schema_version: 1,
        host_id: "host".into(),
        sources: vec![
            Source {
                id: "a".into(),
                adapter: "codex-jsonl".into(),
                paths: vec![a.into()],
            },
            Source {
                id: "b".into(),
                adapter: "codex-jsonl".into(),
                paths: vec![b.into()],
            },
        ],
        model_aliases: BTreeMap::new(),
        model_name_policy: Some(ModelNamePolicy {
            mode: "pattern".into(),
            sources: BTreeMap::from([("a".into(), vec!["gpt-".into()])]),
        }),
    }
}
#[test]
fn actual_collector_source_isolation_cache_and_alias() {
    let t = tempfile::tempdir().unwrap();
    let a = t.path().join("a");
    let b = t.path().join("b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    let fa = a.join("one.jsonl");
    let fb = b.join("two.jsonl");
    fs::write(&fa, fixture("gpt-6.1-terra")).unwrap();
    fs::write(&fb, fixture("gpt-6.1-terra")).unwrap();
    let mut c = cfg(&a, &b);
    let cache = t.path().join("cache.json");
    let cold = collect(&c, Some(&cache)).unwrap();
    let warm = collect(&c, Some(&cache)).unwrap();
    assert_eq!(
        cold.rows.iter().find(|r| r.source_id == "a").unwrap().model,
        "gpt-6.1-terra"
    );
    assert_eq!(
        cold.rows.iter().find(|r| r.source_id == "b").unwrap().model,
        "unknown"
    );
    assert_eq!(
        serde_json::to_value(&cold.rows).unwrap(),
        serde_json::to_value(&warm.rows).unwrap()
    );
    assert_eq!(warm.sources.iter().map(|s| s.cache_hits).sum::<u64>(), 2);
    let mut v: Value = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
    let key = v["files"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, v)| v["parsed"]["records"][0]["model"] == "unknown")
        .unwrap()
        .0
        .clone();
    v["files"][&key]["parsed"]["records"][0]["model"] = json!("gpt-6.1-terra");
    fs::write(&cache, serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!(
        collect(&c, Some(&cache)).unwrap_err().0,
        "invalid_cache_projection"
    );
    c.model_aliases
        .insert("gpt-6.1-terra".into(), "mapped".into());
    let aliased = collect(&c, Some(&cache)).unwrap();
    assert!(aliased.rows.iter().all(|r| r.model == "mapped"));
}
#[test]
fn rule_boundaries_and_invalid_policy() {
    let c = cfg(std::path::Path::new("/a"), std::path::Path::new("/b"));
    let r = c.resolver("a");
    for name in ["gpt-6.1-terra", "gpt-6-private"] {
        assert!(r.automatic(name));
    }
    for name in [
        "gpt-private",
        "gpt-",
        "gpt-6/slash",
        "Gpt-6",
        "unknown",
        &format!("gpt-6{}", "a".repeat(60)),
    ] {
        assert!(!r.automatic(name));
    }
    assert!(c.validate_policy().is_ok());
    for policy in [
        json!({"mode":"pattern","sources":{"a":["gpt-* "]}}),
        json!({"mode":"pattern","sources":{"missing":["gpt-"]}}),
        json!({"mode":"pattern","sources":{"a":["gpt-","gpt-"]}}),
        json!({"mode":"explicit","sources":{}}),
    ] {
        let mut value = serde_json::to_value(&c).unwrap();
        value["model_name_policy"] = policy;
        let parsed: Config = serde_json::from_value(value).unwrap();
        assert!(parsed.validate_policy().is_err());
    }
    let base = r#"{"schema_version":1,"host_id":"x","sources":[],"model_aliases":{},"model_name_policy":null}"#;
    assert!(serde_json::from_str::<Config>(base).is_err());
    let dup = r#"{"schema_version":1,"host_id":"x","sources":[],"model_name_policy":{"mode":"pattern","sources":{"a":["gpt-"],"a":["gpt-"]}}}"#;
    assert!(serde_json::from_str::<Config>(dup).is_err());
}
#[test]
fn legacy_config_serialization_and_fingerprint_are_stable() {
    let original = r#"{"schema_version":1,"host_id":"host","sources":[{"id":"a","adapter":"codex-jsonl","paths":["/fixture"]}],"model_aliases":{"raw":"label"}}"#;
    let c: Config = serde_json::from_str(original).unwrap();
    assert_eq!(serde_json::to_string(&c).unwrap(), original);
    assert_eq!(
        hash(serde_json::to_string(&c).unwrap().as_bytes()),
        "9a3e81bc7c14b2344b27d829f87f03c366f2ac37dd8c09d444c90572b45e2891"
    );
}
