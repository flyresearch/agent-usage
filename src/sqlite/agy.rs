//! agy field projection reviewed against the pinned MIT tokscale-core adapter.
//! Unlike the reference, errors are not empty results and date never uses mtime
//! or session creation as a substitute for a missing generation timestamp.
use super::{open_readonly, wire::Message};
use crate::{
    parser::{Parsed, Record},
    protocol::*,
};
use chrono::{DateTime, FixedOffset, Utc};
use rusqlite::TransactionBehavior;
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Time {
    Unique(i64),
    Ambiguous,
}
fn insert_time<K: Ord>(map: &mut BTreeMap<K, Time>, key: K, time: i64) {
    map.entry(key)
        .and_modify(|old| {
            if *old != Time::Unique(time) {
                *old = Time::Ambiguous;
            }
        })
        .or_insert(Time::Unique(time));
}
fn timestamp(message: Option<Message<'_>>) -> Result<Option<i64>> {
    let Some(m) = message else {
        return Ok(None);
    };
    let seconds = m.number(1)?.unwrap_or(0);
    let nanos = m.number(2)?.unwrap_or(0);
    if seconds == 0 || seconds > i64::MAX as u64 || nanos >= 1_000_000_000 {
        return Ok(None);
    }
    Ok(DateTime::<Utc>::from_timestamp(seconds as i64, nanos as u32).map(|t| t.timestamp_millis()))
}
fn token(m: &Message<'_>, n: u32) -> Result<u64> {
    let value = m.number(n)?.unwrap_or(0);
    if value > i64::MAX as u64 {
        return Err(Error("invalid_token_field"));
    }
    Ok(value)
}

pub fn parse_file(
    path: &Path,
    aliases: &BTreeMap<String, String>,
    file_key: &str,
) -> Result<(Parsed, u64)> {
    let mut connection = open_readonly(path)?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(|_| Error("sqlite_transaction_failed"))?;
    let has_steps: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='steps')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error("sqlite_schema_failed"))?;
    let has_usage: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='gen_metadata')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error("sqlite_schema_failed"))?;
    if !has_usage {
        return Err(Error("unsupported_sqlite_schema"));
    }
    let mut by_response = BTreeMap::new();
    let mut by_index = BTreeMap::new();
    let mut bytes_read = 0u64;
    if has_steps {
        let mut stmt = tx
            .prepare("SELECT metadata FROM steps WHERE step_type=15 AND metadata IS NOT NULL")
            .map_err(|_| Error("unsupported_sqlite_schema"))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(|_| Error("sqlite_read_failed"))?;
        for row in rows {
            let blob = row.map_err(|_| Error("invalid_sqlite_field"))?;
            bytes_read += blob.len() as u64;
            let m = Message::parse(&blob)?;
            let Some(time) = timestamp(m.message(1)?)? else {
                continue;
            };
            if let Some(response) = m.message(9)? {
                if let Some(id) = response.bytes(11)?.filter(|b| !b.is_empty()) {
                    insert_time(&mut by_response, hash(id), time);
                }
            }
            if let Some(generation) = m.message(20)? {
                if let Some(index) = generation.number(3)? {
                    insert_time(&mut by_index, index, time);
                }
            }
        }
    }
    let mut out = Parsed::default();
    let mut records: BTreeMap<String, Record> = BTreeMap::new();
    {
        let mut stmt = tx
            .prepare("SELECT idx,data FROM gen_metadata ORDER BY idx")
            .map_err(|_| Error("unsupported_sqlite_schema"))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))
            .map_err(|_| Error("sqlite_read_failed"))?;
        for row in rows {
            let (index, blob) = row.map_err(|_| Error("invalid_sqlite_field"))?;
            if index < 0 {
                return Err(Error("invalid_generation_index"));
            }
            bytes_read = bytes_read
                .checked_add(blob.len() as u64)
                .ok_or(Error("byte_count_overflow"))?;
            let root = Message::parse(&blob)?;
            let Some(chat) = root.message(1)? else {
                return Err(Error("unsupported_agy_schema"));
            };
            let Some(usage) = chat.message(4)? else {
                continue;
            };
            if usage.is_empty() {
                continue;
            }
            out.coverage.usage_records += 1;
            out.coverage.unsupported_fields += usage.unknown_fields(&[1, 2, 3, 5, 9, 10, 11]);
            let tokens = Tokens {
                input: token(&usage, 1)?
                    .checked_add(token(&usage, 2)?)
                    .ok_or(Error("token_overflow"))?,
                cache_read: token(&usage, 5)?,
                cache_write: 0,
                output: token(&usage, 9)?,
                reasoning: token(&usage, 10)?,
            };
            tokens.total()?;
            let output = tokens
                .output
                .checked_add(tokens.reasoning)
                .ok_or(Error("token_overflow"))?;
            if usage.number(3)?.is_some_and(|reported| reported != output) {
                return Err(Error("agy_output_total_mismatch"));
            }
            if output > 0 && usage.number(3)?.is_none() {
                return Err(Error("unsupported_agy_schema"));
            }
            let response = usage.string(11)?.filter(|s| !s.trim().is_empty());
            let raw_model = chat.string(19)?.filter(|s| !s.trim().is_empty());
            if response.is_none() && raw_model.is_none() {
                // A placeholder can already contain fixed input counters before
                // a model response exists. Do not claim those as known usage.
                out.coverage.unclassified_usage_records += 1;
                continue;
            }
            let native = if let Some(meta) = chat.message(9)? {
                timestamp(meta.message(4)?)?
            } else {
                None
            };
            let by_id = response
                .and_then(|r| by_response.get(&hash(r.as_bytes())))
                .copied();
            let by_idx = by_index.get(&(index as u64)).copied();
            let joined = match (by_id, by_idx) {
                (Some(Time::Ambiguous), _) | (_, Some(Time::Ambiguous)) => Some(Time::Ambiguous),
                (Some(Time::Unique(a)), Some(Time::Unique(b))) if a != b => Some(Time::Ambiguous),
                (a, b) => a.or(b),
            };
            let time = if let Some(t) = native {
                Some(t)
            } else {
                match joined {
                    Some(Time::Unique(t)) => Some(t),
                    Some(Time::Ambiguous) => {
                        out.coverage.ambiguous_time_records += 1;
                        None
                    }
                    None => None,
                }
            };
            let day = time
                .and_then(DateTime::<Utc>::from_timestamp_millis)
                .map(|t| {
                    t.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap())
                        .format("%Y-%m-%d")
                        .to_string()
                });
            if day.is_none() {
                out.coverage.missing_time_records += 1;
            }
            let model = raw_model
                .and_then(|m| aliases.get(m))
                .cloned()
                .unwrap_or_else(|| {
                    out.coverage.unknown_model_records += 1;
                    "unknown".into()
                });
            let key = if let Some(id) = response {
                hash(&serde_json::to_vec(&("agy-response", id)).unwrap())
            } else {
                out.coverage.missing_identity_records += 1;
                hash(&serde_json::to_vec(&("agy-generation", file_key, index)).unwrap())
            };
            let record = Record {
                messages: if response.is_some() {
                    MessageCount::one()
                } else {
                    MessageCount::default()
                },
                key: key.clone(),
                model,
                model_key: hash(raw_model.unwrap_or("").as_bytes()),
                message_key: None,
                request_present: false,
                cache_write_uncertain: false,
                claude_usage_stage: 0,
                day,
                hour: time.and_then(|t| crate::parser::event_hour(&serde_json::json!(t))),
                execution_role: None,
                tokens,
                interval: None,
                time_basis: None,
                usage_kind: None,
                backend: None,
                provider: None,
            };
            if let Some(old) = records.get(&key) {
                if old.tokens != record.tokens
                    || old.day != record.day
                    || old.hour != record.hour
                    || old.model_key != record.model_key
                {
                    return Err(Error("conflicting_response_replay"));
                }
            } else {
                records.insert(key, record);
            }
        }
    }
    tx.rollback()
        .map_err(|_| Error("sqlite_transaction_failed"))?;
    out.records = records.into_values().collect();
    Ok((out, bytes_read))
}
