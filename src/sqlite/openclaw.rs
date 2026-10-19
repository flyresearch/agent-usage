//! Explicit runtime linkage fences Codex mirrors; transcript APIs identify the
//! supported non-Codex paths. Window labels and model names never decide this.
use super::open_readonly;
use crate::{
    parser::{Parsed, Record},
    protocol::*,
};
use chrono::{DateTime, FixedOffset, Utc};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
const LIMIT: usize = 16 * 1024 * 1024;
fn json(bytes: &[u8]) -> Result<Value> {
    if bytes.len() > LIMIT {
        return Err(Error("sqlite_event_too_large"));
    }
    serde_json::from_slice(bytes).map_err(|_| Error("invalid_sqlite_json"))
}
fn n(v: &Value, key: &str, required: bool) -> Result<u64> {
    match v.get(key) {
        None if !required => Ok(0),
        Some(v) => v.as_u64().ok_or(Error("invalid_token_field")),
        _ => Err(Error("missing_token_field")),
    }
}
fn tokens(v: &Value) -> Result<Tokens> {
    if !v.is_object() {
        return Err(Error("invalid_token_field"));
    }
    let reasoning = n(v, "reasoningTokens", false)?;
    let t = Tokens {
        input: n(v, "input", true)?,
        output: n(v, "output", true)?
            .checked_sub(reasoning)
            .ok_or(Error("invalid_reasoning_subset"))?,
        cache_read: n(v, "cacheRead", false)?,
        cache_write: n(v, "cacheWrite", false)?,
        reasoning,
    };
    let total = t.total()?;
    for key in ["total", "totalTokens"] {
        if v.get(key).is_some() && n(v, key, true)? != total {
            return Err(Error("invalid_total_tokens"));
        }
    }
    Ok(t)
}
fn day(v: &Value, cov: &mut Coverage) -> Option<String> {
    let time = if let Some(s) = v.as_str() {
        DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|t| t.with_timezone(&Utc))
    } else {
        v.as_i64().and_then(DateTime::from_timestamp_millis)
    };
    let result = time.map(|t| {
        t.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap())
            .format("%Y-%m-%d")
            .to_string()
    });
    if result.is_none() {
        cov.missing_time_records += 1;
    }
    result
}
pub fn parse_file(
    path: &Path,
    aliases: &BTreeMap<String, String>,
    file_key: &str,
    backend_threads: &BTreeSet<String>,
) -> Result<(Parsed, u64)> {
    let mut conn = open_readonly(path)?;
    let tx = conn
        .transaction()
        .map_err(|_| Error("sqlite_transaction_failed"))?;
    let mut out = Parsed::default();
    let mut bytes_read = 0u64;
    let mut runs: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut missing = BTreeSet::new();
    {
        let mut stmt=tx.prepare("SELECT run_id,length(CAST(event_json AS BLOB)),event_json,session_id FROM trajectory_runtime_events").map_err(|_|Error("unsupported_sqlite_schema"))?;
        let mut rows = stmt.query([]).map_err(|_| Error("sqlite_read_failed"))?;
        while let Some(row) = rows.next().map_err(|_| Error("sqlite_read_failed"))? {
            let len: i64 = row.get(1).map_err(|_| Error("invalid_sqlite_field"))?;
            if len < 0 || len as u64 > LIMIT as u64 {
                return Err(Error("sqlite_event_too_large"));
            }
            let raw: String = row.get(2).map_err(|_| Error("invalid_sqlite_field"))?;
            bytes_read += raw.len() as u64;
            let o = json(raw.as_bytes())?;
            if ![Some("model.completed"), Some("session.started")].contains(&o["type"].as_str()) {
                continue;
            }
            if o["type"] == "model.completed" && o["data"].get("usage").is_some() {
                out.coverage.runtime_usage_records += 1;
            }
            // A runtime Codex thread is evidence of a backend, not extra usage.
            if let Some(thread) = o["data"]["threadId"].as_str().filter(|s| !s.is_empty()) {
                let run: String = row.get(0).map_err(|_| Error("invalid_sqlite_field"))?;
                if run.is_empty() {
                    return Err(Error("missing_run_identity"));
                }
                let thread = hash(thread.as_bytes());
                if !backend_threads.contains(&thread) {
                    missing.insert(thread.clone());
                }
                let session: String = row.get(3).map_err(|_| Error("invalid_sqlite_field"))?;
                if runs
                    .insert((session, run), thread.clone())
                    .is_some_and(|old| old != thread)
                {
                    return Err(Error("conflicting_backend_linkage"));
                }
            }
        }
    }
    out.coverage.missing_backend_threads = missing.len() as u64;
    let mut seen = BTreeMap::<String, Record>::new();
    {
        let mut stmt=tx.prepare("SELECT session_id,seq,length(CAST(event_json AS BLOB)),length(event_zstd),event_json,event_zstd,event_utf8_bytes FROM transcript_events ORDER BY session_id,seq").map_err(|_|Error("unsupported_sqlite_schema"))?;
        let mut rows = stmt.query([]).map_err(|_| Error("sqlite_read_failed"))?;
        while let Some(row) = rows.next().map_err(|_| Error("sqlite_read_failed"))? {
            let plain: Option<i64> = row.get(2).map_err(|_| Error("invalid_sqlite_field"))?;
            let compressed: Option<i64> = row.get(3).map_err(|_| Error("invalid_sqlite_field"))?;
            if plain
                .into_iter()
                .chain(compressed)
                .any(|n| n < 0 || n as u64 > LIMIT as u64)
            {
                return Err(Error("sqlite_event_too_large"));
            }
            let data = if plain.is_some() {
                row.get::<_, String>(4)
                    .map_err(|_| Error("invalid_sqlite_field"))?
                    .into_bytes()
            } else {
                let expected: i64 = row.get(6).map_err(|_| Error("invalid_sqlite_field"))?;
                if expected <= 0 || expected as u64 > LIMIT as u64 {
                    return Err(Error("sqlite_event_too_large"));
                }
                let blob: Vec<u8> = row.get(5).map_err(|_| Error("invalid_sqlite_field"))?;
                let decoded = zstd::bulk::decompress(&blob, expected as usize)
                    .map_err(|_| Error("invalid_zstd_event"))?;
                if decoded.len() != expected as usize {
                    return Err(Error("invalid_zstd_size"));
                }
                decoded
            };
            bytes_read += data.len() as u64;
            let o = json(&data)?;
            if o["type"] != "message" {
                continue;
            }
            let m = &o["message"];
            let Some(usage) = m.get("usage") else {
                continue;
            };
            out.coverage.usage_records += 1;
            let t = tokens(usage)?;
            let sid: String = row.get(0).map_err(|_| Error("invalid_sqlite_field"))?;
            let run = m["__openclaw"]["runId"].as_str();
            if let Some(thread) = run.and_then(|r| runs.get(&(sid.clone(), r.to_string()))) {
                if backend_threads.contains(thread) {
                    out.coverage.mirrored_usage_records += 1;
                } else {
                    out.coverage.unclassified_usage_records += 1;
                }
                continue;
            }
            let api = m["api"].as_str().unwrap_or("");
            // Only explicit message transport APIs qualify these paths. An
            // OpenAI Responses model name/provider alone cannot prove harness.
            if m["role"] != "assistant"
                || ![
                    "openai-completions",
                    "anthropic-messages",
                    "google-generative-ai",
                ]
                .contains(&api)
                || m["__openclaw"].get("mirrorOrigin").is_some()
            {
                out.coverage.unclassified_usage_records += 1;
                continue;
            }
            let sid: String = row.get(0).map_err(|_| Error("invalid_sqlite_field"))?;
            let seq: i64 = row.get(1).map_err(|_| Error("invalid_sqlite_field"))?;
            if sid.is_empty() || seq < 0 {
                return Err(Error("missing_event_identity"));
            }
            let key = if let Some(id) = m["idempotencyKey"].as_str().filter(|s| !s.is_empty()) {
                hash(&serde_json::to_vec(&("openclaw-message", sid, id)).unwrap())
            } else {
                hash(&serde_json::to_vec(&("openclaw-event", file_key, sid, seq)).unwrap())
            };
            let raw_model = m["model"].as_str().unwrap_or("");
            let model = aliases.get(raw_model).cloned().unwrap_or_else(|| {
                out.coverage.unknown_model_records += 1;
                "unknown".into()
            });
            let record = Record {
                messages: MessageCount::one(),
                key: key.clone(),
                model,
                model_key: hash(raw_model.as_bytes()),
                message_key: None,
                request_present: false,
                cache_write_uncertain: false,
                claude_usage_stage: 0,
                day: day(&m["timestamp"], &mut out.coverage),
                hour: crate::parser::event_hour(&m["timestamp"]),
                execution_role: None,
                tokens: t,
                interval: None,
                time_basis: None,
                usage_kind: None,
                backend: Some(api.into()),
                provider: m["provider"]
                    .as_str()
                    .map(|v| format!("provider-{}", &hash(v.as_bytes())[..12])),
            };
            if let Some(old) = seen.get(&key) {
                if old.tokens != record.tokens
                    || old.day != record.day
                    || old.hour != record.hour
                    || old.model_key != record.model_key
                    || old.backend != record.backend
                    || old.provider != record.provider
                {
                    return Err(Error("conflicting_message_replay"));
                }
                out.coverage.mirrored_usage_records += 1;
            } else {
                seen.insert(key, record);
            }
        }
    }
    out.records = seen.into_values().collect();
    tx.rollback()
        .map_err(|_| Error("sqlite_transaction_failed"))?;
    Ok((out, bytes_read))
}
