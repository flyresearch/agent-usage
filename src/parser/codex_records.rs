//! Response-level Codex records (source contract pinned in docs/codex-response-usage.md).
//! No provider SDK or upstream implementation is linked/copied.
use super::{
    canonical, codex_usage, day, model, number, string_event_hour, string_event_second,
    unknown_fields, Parsed, Record,
};
use crate::protocol::*;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) struct Context<'a> {
    pub execution_role: Option<&'a str>,
    pub thread: Option<&'a str>,
    pub turn: Option<&'a str>,
    pub model: Option<&'a str>,
    pub aliases: &'a ModelResolver<'a>,
}
#[derive(Default)]
pub(super) struct ResponseUsage {
    last: Option<[u64; 5]>,
    signatures: BTreeMap<String, (String, usize)>,
    compacted: bool,
    last_record: Option<usize>,
    // Hashed (response, turn) identity of last_record.
    last_identity: Option<(String, String)>,
    // Compaction record awaiting the turn context Codex re-emits for its turn.
    pending: Option<(usize, String)>,
}
fn attribute(r: &mut Record, raw: Option<&str>, aliases: &ModelResolver<'_>, cov: &mut Coverage) {
    let was_unknown = r.model == "unknown";
    match raw.and_then(|m| aliases.get(m)) {
        Some(label) => {
            if was_unknown {
                cov.unknown_model_records = cov.unknown_model_records.saturating_sub(1);
            }
            r.model = label.to_owned();
            r.model_key = hash(raw.unwrap_or("").as_bytes());
        }
        None => {
            if !was_unknown {
                cov.unknown_model_records += 1;
            }
            r.model = "unknown".into();
            r.model_key = hash(b"");
        }
    }
}
impl ResponseUsage {
    pub fn active(&self) -> bool {
        self.last.is_some()
    }
    pub fn compacted(
        &mut self,
        p: &Value,
        aliases: &ModelResolver<'_>,
        records: &mut [Record],
        cov: &mut Coverage,
    ) {
        self.compacted = true;
        self.pending = None;
        // Compaction may use a different model than the conversation context.
        // Attribute it only when the compacted event names the latest response;
        // the model comes from the recorded previous turn settings, otherwise
        // from the turn context re-emitted for that same turn (see turn_context).
        let turn = p["compaction_response_id"]
            .as_str()
            .and_then(|id| {
                self.last_identity
                    .as_ref()
                    .filter(|(response, _)| *response == hash(id.as_bytes()))
            })
            .map(|(_, turn)| turn.clone());
        let raw = turn
            .as_ref()
            .and_then(|_| p["resume_metadata"]["previous_turn_settings"]["model"].as_str())
            .filter(|s| !s.is_empty());
        if let Some(i) = self.last_record.filter(|i| *i < records.len()) {
            attribute(&mut records[i], raw, aliases, cov);
            if raw.is_none() {
                self.pending = turn.map(|t| (i, t));
            }
        }
    }
    pub fn turn_context(
        &mut self,
        turn: Option<&str>,
        raw: Option<&str>,
        aliases: &ModelResolver<'_>,
        records: &mut [Record],
        cov: &mut Coverage,
    ) {
        if let Some((i, expected)) = self.pending.take() {
            if turn == Some(expected.as_str()) && i < records.len() {
                attribute(&mut records[i], raw, aliases, cov);
            }
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
            let previous = &mut records[old.1];
            if previous.event_time != string_event_second(&v["timestamp"]) {
                previous.event_time = None;
            }
            let next_day = day(v, cov);
            if previous.day != next_day {
                previous.day = None;
                previous.hour = None;
            }
            if previous.hour != string_event_hour(&v["timestamp"]) {
                previous.hour = None;
            }
            if previous.execution_role.as_deref() != context.execution_role {
                previous.execution_role = None;
            }
            self.last = Some(usage);
            self.last_record = Some(old.1);
            self.last_identity = Some((hash(response_id.as_bytes()), hash(turn_id.as_bytes())));
            self.pending = None;
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
            hour: string_event_hour(&v["timestamp"]),
            event_time: string_event_second(&v["timestamp"]),
            execution_role: context.execution_role.map(str::to_owned),
            tokens: canonical(usage)?,
            interval: None,
            time_basis: None,
            usage_kind: None,
            backend: None,
            provider: None,
        });
        self.last_record = Some(records.len() - 1);
        self.last_identity = Some((hash(response_id.as_bytes()), hash(turn_id.as_bytes())));
        self.pending = None;
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
