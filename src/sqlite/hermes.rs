//! Hermes uses model usage as the authority (including auxiliary calls).
//! Sessions only supply a nonnegative, previously uncovered main-call residual.
use super::open_readonly;
use crate::{
    parser::{Parsed, Record},
    protocol::*,
};
use chrono::{DateTime, FixedOffset, Utc};
use rusqlite::{Row, TransactionBehavior};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Copy, Default)]
struct Raw([u64; 5]); // input, inclusive output, cache read, cache write, reasoning
impl Raw {
    fn read(row: &Row<'_>, offset: usize) -> Result<Self> {
        let mut a = [0; 5];
        for (i, value) in a.iter_mut().enumerate() {
            let n: i64 = row
                .get(offset + i)
                .map_err(|_| Error("invalid_sqlite_field"))?;
            *value = u64::try_from(n).map_err(|_| Error("invalid_token_field"))?;
        }
        let value = Self(a);
        value.tokens()?;
        Ok(value)
    }
    fn tokens(self) -> Result<Tokens> {
        let t = Tokens {
            input: self.0[0],
            output: self.0[1]
                .checked_sub(self.0[4])
                .ok_or(Error("invalid_reasoning_subset"))?,
            cache_read: self.0[2],
            cache_write: self.0[3],
            reasoning: self.0[4],
        };
        t.total()?;
        Ok(t)
    }
    fn add(self, other: Self) -> Result<Self> {
        let mut a = self.0;
        for (n, b) in a.iter_mut().zip(other.0) {
            *n = n.checked_add(b).ok_or(Error("token_overflow"))?;
        }
        Ok(Self(a))
    }
    fn difference(self, other: Self) -> Option<Self> {
        let mut a = self.0;
        for (n, b) in a.iter_mut().zip(other.0) {
            *n = n.checked_sub(b)?;
        }
        Some(Self(a))
    }
}
fn instant(value: Option<f64>) -> Option<DateTime<Utc>> {
    let v = value?;
    if !v.is_finite() || v <= 0.0 || v * 1000.0 > i64::MAX as f64 {
        return None;
    }
    DateTime::from_timestamp_millis((v * 1000.0).floor() as i64)
}
fn interval(
    first: Option<f64>,
    last: Option<f64>,
    cov: &mut Coverage,
) -> Result<(Option<String>, Option<UsageInterval>, String)> {
    let first = instant(first);
    let last = instant(last);
    if matches!((&first,&last),(Some(a),Some(b)) if a>b) {
        return Err(Error("invalid_usage_interval"));
    }
    let day = match (&first, &last) {
        (Some(a), Some(b)) => {
            let tz = FixedOffset::east_opt(8 * 3600).unwrap();
            let a = a.with_timezone(&tz).format("%Y-%m-%d").to_string();
            let b = b.with_timezone(&tz).format("%Y-%m-%d").to_string();
            if a == b {
                Some(a)
            } else {
                None
            }
        }
        _ => {
            cov.missing_time_records += 1;
            None
        }
    };
    let bounds = if first.is_some() || last.is_some() {
        Some(UsageInterval {
            start: first.map(|t| t.to_rfc3339()),
            end: last.map(|t| t.to_rfc3339()),
        })
    } else {
        None
    };
    let basis = if day.is_some() {
        "aggregate_interval"
    } else if bounds.is_some() {
        "unallocated_interval"
    } else {
        "unallocated"
    };
    Ok((day, bounds, basis.into()))
}
fn model(raw: Option<&str>, aliases: &BTreeMap<String, String>, cov: &mut Coverage) -> String {
    raw.and_then(|s| aliases.get(s))
        .cloned()
        .unwrap_or_else(|| {
            cov.unknown_model_records += 1;
            "unknown".into()
        })
}
pub fn parse_file(
    path: &Path,
    aliases: &BTreeMap<String, String>,
    file_key: &str,
) -> Result<(Parsed, u64)> {
    let mut conn = open_readonly(path)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(|_| Error("sqlite_transaction_failed"))?;
    let mut out = Parsed::default();
    let mut main: BTreeMap<String, Raw> = BTreeMap::new();
    let mut model_sessions = BTreeSet::new();
    let mut seen_sessions = BTreeSet::new();
    {
        let has_calls = {
            let mut stmt = tx
                .prepare("PRAGMA table_info(session_model_usage)")
                .map_err(|_| Error("unsupported_sqlite_schema"))?;
            let columns = stmt
                .query_map([], |r| r.get::<_, String>(1))
                .map_err(|_| Error("sqlite_read_failed"))?;
            columns
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| Error("sqlite_read_failed"))?
                .iter()
                .any(|c| c == "api_call_count")
        };
        let query=format!("SELECT session_id,model,task,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,reasoning_tokens,first_seen,last_seen,{} FROM session_model_usage ORDER BY session_id,model,task,rowid",if has_calls {"api_call_count"} else {"NULL"});
        let mut stmt = tx
            .prepare(&query)
            .map_err(|_| Error("unsupported_sqlite_schema"))?;
        let mut rows = stmt.query([]).map_err(|_| Error("sqlite_read_failed"))?;
        let mut ordinal = 0u64;
        while let Some(row) = rows.next().map_err(|_| Error("sqlite_read_failed"))? {
            ordinal += 1;
            let sid: String = row.get(0).map_err(|_| Error("invalid_sqlite_field"))?;
            if sid.is_empty() {
                return Err(Error("missing_session_identity"));
            }
            model_sessions.insert(sid.clone());
            let raw_model: Option<String> =
                row.get(1).map_err(|_| Error("invalid_sqlite_field"))?;
            let task: Option<String> = row.get(2).map_err(|_| Error("invalid_sqlite_field"))?;
            let counts = Raw::read(row, 3)?;
            let tokens = counts.tokens()?;
            let calls: Option<i64> = row.get(10).map_err(|_| Error("invalid_sqlite_field"))?;
            let messages = match calls {
                Some(n) => MessageCount {
                    known: u64::try_from(n).map_err(|_| Error("invalid_message_count"))?,
                    complete: true,
                },
                None => MessageCount::default(),
            };
            let auxiliary = task.as_deref().is_some_and(|s| !s.is_empty());
            if !auxiliary {
                let value = main.entry(sid.clone()).or_default();
                *value = value.add(counts)?;
            }
            out.coverage.usage_records += 1;
            if auxiliary {
                out.coverage.auxiliary_usage_records += 1;
            }
            if tokens.total()? == 0 && messages.known == 0 {
                continue;
            }
            let first: Option<f64> = row.get(8).map_err(|_| Error("invalid_sqlite_field"))?;
            let last: Option<f64> = row.get(9).map_err(|_| Error("invalid_sqlite_field"))?;
            let (day, bounds, basis) = interval(first, last, &mut out.coverage)?;
            out.records.push(Record {
                hour: None,
                execution_role: None,
                messages,
                key: hash(&serde_json::to_vec(&("hermes-model", file_key, sid, ordinal)).unwrap()),
                model: model(raw_model.as_deref(), aliases, &mut out.coverage),
                model_key: hash(raw_model.as_deref().unwrap_or("").as_bytes()),
                message_key: None,
                request_present: false,
                cache_write_uncertain: false,
                claude_usage_stage: 0,
                day,
                tokens,
                interval: bounds,
                time_basis: Some(basis),
                usage_kind: Some(if auxiliary { "auxiliary" } else { "main" }.into()),
                backend: None,
                provider: None,
            });
        }
    }
    {
        let mut stmt=tx.prepare("SELECT id,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,reasoning_tokens,started_at,ended_at FROM sessions ORDER BY id").map_err(|_|Error("unsupported_sqlite_schema"))?;
        let mut rows = stmt.query([]).map_err(|_| Error("sqlite_read_failed"))?;
        while let Some(row) = rows.next().map_err(|_| Error("sqlite_read_failed"))? {
            let id: String = row.get(0).map_err(|_| Error("invalid_sqlite_field"))?;
            if id.is_empty() {
                return Err(Error("missing_session_identity"));
            }
            if !seen_sessions.insert(id.clone()) {
                return Err(Error("duplicate_session_identity"));
            }
            model_sessions.remove(&id);
            let counts = Raw::read(row, 1)?;
            let Some(residual) = counts.difference(main.remove(&id).unwrap_or_default()) else {
                out.coverage.negative_residual_records += 1;
                continue;
            };
            let tokens = match residual.tokens() {
                Ok(t) => t,
                Err(_) => {
                    out.coverage.negative_residual_records += 1;
                    continue;
                }
            };
            if tokens.total()? == 0 {
                continue;
            }
            let first: Option<f64> = row.get(6).map_err(|_| Error("invalid_sqlite_field"))?;
            let last: Option<f64> = row.get(7).map_err(|_| Error("invalid_sqlite_field"))?;
            let (day, bounds, basis) = interval(first, last, &mut out.coverage)?;
            out.coverage.residual_usage_records += 1;
            out.coverage.usage_records += 1;
            // sessions.model can be the latest model of a multi-model session;
            // it does not prove which model produced an uncovered residual.
            out.records.push(Record {
                hour: None,
                execution_role: None,
                messages: MessageCount::default(),
                key: hash(&serde_json::to_vec(&("hermes-residual", file_key, id)).unwrap()),
                model: model(None, aliases, &mut out.coverage),
                model_key: hash(b""),
                message_key: None,
                request_present: false,
                cache_write_uncertain: false,
                claude_usage_stage: 0,
                day,
                tokens,
                interval: bounds,
                time_basis: Some(basis),
                usage_kind: Some("residual".into()),
                backend: None,
                provider: None,
            });
        }
    }
    out.coverage.orphan_usage_records = model_sessions.len() as u64;
    tx.rollback()
        .map_err(|_| Error("sqlite_transaction_failed"))?;
    // No source text/BLOB is selected; exact physical page IO is not reported.
    Ok((out, 0))
}
