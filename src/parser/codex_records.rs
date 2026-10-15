//! Response-level Codex records (source contract pinned in docs/codex-response-usage.md).
//! No provider SDK or upstream implementation is linked/copied.
use super::{canonical, codex_usage, day, model, number, unknown_fields, Parsed, Record};
use crate::protocol::*;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) struct Context<'a> {
    pub thread: Option<&'a str>,
    pub turn: Option<&'a str>,
    pub model: Option<&'a str>,
    pub aliases: &'a BTreeMap<String, String>,
}
#[derive(Default)]
pub(super) struct ResponseUsage {
    last: Option<[u64; 5]>,
    signatures: BTreeMap<String, (String, usize)>,
    compacted: bool,
    last_record: Option<usize>,
}
impl ResponseUsage {
    pub fn active(&self) -> bool {
        self.last.is_some()
    }
    pub fn compacted(&mut self, records: &mut [Record], cov: &mut Coverage) {
        self.compacted = true;
        // Compaction may use a different model than the conversation context.
        // The response usage record has no model, so do not claim that attribution.
        if let Some(r) = self.last_record.and_then(|i| records.get_mut(i)) {
            if r.model != "unknown" {
                cov.unknown_model_records += 1;
            }
            r.model = "unknown".into();
            r.model_key = hash(b"");
        }
    }
    pub fn record(&mut self, v: &Value, context: Context<'_>, out: &mut Parsed) -> Result<()> {
        let records = &mut out.records;
        let cov = &mut out.coverage;
        let p = &v["payload"];
        let identity = |key| {
            p.get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or(Error("missing_response_identity"))
        };
        let thread_id = identity("thread_id")?;
        let turn_id = identity("turn_id")?;
        let session_id = identity("session_id")?;
        let root_turn_id = identity("root_turn_id")?;
        let response_id = identity("response_id")?;
        if context
            .thread
            .is_some_and(|t| t != hash(thread_id.as_bytes()))
        {
            return Err(Error("conflicting_response_thread"));
        }
        let usage = codex_usage(&p["usage"], cov)?;
        // A response record must contain the actual parent buckets, not only a
        // reported total. The latter has no reliable billable decomposition.
        number(&p["usage"], "input_tokens", true)?;
        number(&p["usage"], "output_tokens", true)?;
        let reported = number(&p["usage"], "total_tokens", true)?;
        if reported != canonical(usage)?.total()? {
            return Err(Error("unclassified_response_total"));
        }
        for name in ["turn_token_usage", "thread_token_usage"] {
            let cumulative = codex_usage(&p[name], cov)?;
            if usage.iter().zip(cumulative).any(|(a, b)| *a > b) {
                return Err(Error("invalid_response_cumulative"));
            }
        }
        unknown_fields(
            p,
            &[
                "thread_id",
                "turn_id",
                "session_id",
                "root_turn_id",
                "response_id",
                "usage",
                "turn_token_usage",
                "thread_token_usage",
            ],
            cov,
        );
        let key = hash(&serde_json::to_vec(&("codex-response", thread_id, response_id)).unwrap());
        let signature = hash(
            &serde_json::to_vec(&(
                turn_id,
                session_id,
                root_turn_id,
                &p["usage"],
                &p["turn_token_usage"],
                &p["thread_token_usage"],
            ))
            .unwrap(),
        );
        cov.usage_records += 1;
        cov.response_usage_records += 1;
        if let Some(old) = self.signatures.get(&key) {
            if old.0 != signature {
                return Err(Error("conflicting_response_replay"));
            }
            self.last = Some(usage);
            self.last_record = Some(old.1);
            return Ok(());
        }
        self.signatures
            .insert(key.clone(), (signature, records.len()));
        self.last = Some(usage);
        self.compacted = false;
        let raw_model = context
            .model
            .filter(|_| context.turn == Some(hash(turn_id.as_bytes()).as_str()));
        records.push(Record {
            messages: MessageCount::one(),
            key,
            model: model(raw_model, context.aliases, cov),
            model_key: hash(raw_model.unwrap_or("").as_bytes()),
            message_key: None,
            request_present: false,
            cache_write_uncertain: false,
            claude_usage_stage: 0,
            day: day(v, cov),
            tokens: canonical(usage)?,
            interval: None,
            time_basis: None,
            usage_kind: None,
            backend: None,
            provider: None,
        });
        self.last_record = Some(records.len() - 1);
        Ok(())
    }
    pub fn mirror(&self, info: &Value, cov: &mut Coverage) -> Result<()> {
        // Totals are context counters and can be rewritten by compaction. They
        // are validated, never added to lifetime response usage.
        if let Some(t) = info.get("total_token_usage").filter(|t| !t.is_null()) {
            codex_usage(t, cov)?;
        }
        let last = info
            .get("last_token_usage")
            .filter(|t| !t.is_null())
            .ok_or(Error("unmatched_response_mirror"))?;
        let usage = codex_usage(last, cov)?;
        number(last, "input_tokens", true)?;
        number(last, "output_tokens", true)?;
        if self.compacted && usage == [0; 5] && number(last, "total_tokens", false)? > 0 {
            cov.context_estimate_records += 1;
            return Ok(());
        }
        if Some(usage) != self.last {
            return Err(Error("unmatched_response_mirror"));
        }
        if last.get("total_tokens").is_some()
            && number(last, "total_tokens", true)? != canonical(usage)?.total()?
        {
            return Err(Error("unmatched_response_mirror"));
        }
        cov.mirrored_usage_records += 1;
        Ok(())
    }
}
