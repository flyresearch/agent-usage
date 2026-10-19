//! Strict local usage projection. Count-selection rules reviewed against the
//! MIT tokscale-core pin in docs/dependency-review.md; no upstream IO/init code.
mod codex_records;
mod workbuddy;
use crate::protocol::*;
use chrono::{DateTime, FixedOffset, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub messages: MessageCount,
    pub key: String,
    pub model: String,
    pub model_key: String,
    // Hashed Claude message identity is needed across files/cache hits too.
    #[serde(default)]
    pub message_key: Option<String>,
    #[serde(default)]
    pub request_present: bool,
    #[serde(default)]
    pub cache_write_uncertain: bool,
    #[serde(default)]
    pub claude_usage_stage: u8,
    pub day: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_role: Option<String>,
    pub tokens: Tokens,
    #[serde(default)]
    pub interval: Option<UsageInterval>,
    #[serde(default)]
    pub time_basis: Option<String>,
    #[serde(default)]
    pub usage_kind: Option<String>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Parsed {
    pub records: Vec<Record>,
    pub coverage: Coverage,
    #[serde(default)]
    pub codex_thread: Option<String>,
}
fn number(v: &Value, name: &str, required: bool) -> Result<u64> {
    match v.get(name) {
        None if !required => Ok(0),
        Some(n) => n.as_u64().ok_or(Error("invalid_token_field")),
        _ => Err(Error("missing_token_field")),
    }
}
fn day(v: &Value, coverage: &mut Coverage) -> Option<String> {
    let date = v
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| {
            d.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap())
                .format("%Y-%m-%d")
                .to_string()
        });
    if date.is_none() {
        coverage.missing_time_records += 1;
    }
    date
}
/// Event timestamps only; aggregate intervals are deliberately not spread over hours.
pub(crate) fn event_hour(value: &Value) -> Option<u8> {
    let time = if let Some(text) = value.as_str() {
        DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|v| v.with_timezone(&chrono::Utc))
    } else {
        value.as_i64().and_then(DateTime::from_timestamp_millis)
    };
    time.map(|v| {
        v.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap())
            .hour() as u8
    })
}
fn string_event_hour(value: &Value) -> Option<u8> {
    value.as_str().and_then(|_| event_hour(value))
}
fn claude_role(value: &Value) -> Option<String> {
    value
        .get("isSidechain")
        .and_then(Value::as_bool)
        .map(|sub| if sub { "subagent" } else { "main" }.into())
}
fn model(raw: Option<&str>, aliases: &BTreeMap<String, String>, cov: &mut Coverage) -> String {
    match raw.and_then(|s| aliases.get(s)) {
        Some(s) => s.clone(),
        None => {
            cov.unknown_model_records += 1;
            "unknown".into()
        }
    }
}
// Raw Codex counters: input and output INCLUDE their child buckets.
fn codex_usage(v: &Value, cov: &mut Coverage) -> Result<[u64; 5]> {
    if !v.is_object() {
        return Err(Error("unsupported_usage_shape"));
    }
    let total_only = v.get("input_tokens").is_none()
        && v.get("output_tokens").is_none()
        && v.get("total_tokens").is_some();
    let i = number(v, "input_tokens", !total_only)?;
    let o = number(v, "output_tokens", !total_only)?;
    let c = if v.get("cached_input_tokens").is_some() {
        let c = number(v, "cached_input_tokens", false)?;
        if v.get("cache_read_input_tokens").is_some()
            && number(v, "cache_read_input_tokens", false)? != c
        {
            return Err(Error("conflicting_cache_fields"));
        }
        c
    } else {
        number(v, "cache_read_input_tokens", false)?
    };
    let r = number(v, "reasoning_output_tokens", false)?;
    let w = number(v, "cache_write_input_tokens", false)?;
    canonical([i, c, o, r, w])?;
    if v.get("total_tokens").is_some()
        && number(v, "total_tokens", true)? < i.checked_add(o).ok_or(Error("token_overflow"))?
    {
        return Err(Error("invalid_reported_total"));
    }
    unknown_fields(
        v,
        &[
            "input_tokens",
            "output_tokens",
            "cached_input_tokens",
            "cache_write_input_tokens",
            "cache_read_input_tokens",
            "reasoning_output_tokens",
            "total_tokens",
        ],
        cov,
    );
    Ok([i, c, o, r, w])
}
fn canonical(a: [u64; 5]) -> Result<Tokens> {
    let t = Tokens {
        input: a[0]
            .checked_sub(a[1])
            .and_then(|remaining| remaining.checked_sub(a[4]))
            .ok_or(Error("invalid_cache_subset"))?,
        cache_read: a[1],
        output: a[2]
            .checked_sub(a[3])
            .ok_or(Error("invalid_reasoning_subset"))?,
        reasoning: a[3],
        cache_write: a[4],
    };
    t.total()?;
    Ok(t)
}
fn unknown_fields(v: &Value, known: &[&str], cov: &mut Coverage) {
    if let Some(map) = v.as_object() {
        cov.unsupported_fields +=
            map.keys().filter(|k| !known.contains(&k.as_str())).count() as u64;
    }
}
fn delta(a: [u64; 5], b: [u64; 5]) -> Option<[u64; 5]> {
    Some([
        a[0].checked_sub(b[0])?,
        a[1].checked_sub(b[1])?,
        a[2].checked_sub(b[2])?,
        a[3].checked_sub(b[3])?,
        a[4].checked_sub(b[4])?,
    ])
}

/// Reads only complete newline-terminated records. An incomplete EOF is retried
/// on the next scan, including when the bytes happen to form valid JSON already.
pub fn parse(
    reader: impl BufRead,
    adapter: &str,
    aliases: &BTreeMap<String, String>,
    file_key: &str,
) -> Result<Parsed> {
    if adapter == "workbuddy-jsonl" {
        return workbuddy::parse(reader, aliases);
    }
    if !["codex-jsonl", "claude-jsonl"].contains(&adapter) {
        return Err(Error("unsupported_adapter"));
    }
    let mut out = Parsed::default();
    let mut previous = None;
    let mut unanchored_usage = false;
    let mut current_model: Option<String> = None;
    let mut execution_role: Option<String> = None;
    let mut context_turn: Option<String> = None;
    let mut task_ids = BTreeSet::new();
    let mut pending_task: Option<String> = None;
    let mut previous_usage_task: Option<String> = None;
    let mut thread_key: Option<String> = None;
    let mut fork_owner: Option<String> = None;
    let mut fork_creation_ms = None;
    let mut fork_own_turn = false;
    let mut fork_owned_legacy = false;
    let mut legacy_unmetered = false;
    let mut parsed_lines = 0u64;
    let mut fork_ancestors = BTreeSet::new();
    let mut fork_links = BTreeMap::new();
    let mut response_usage = codex_records::ResponseUsage::default();
    let mut session_key = file_key.to_string();
    let mut seen_codex = BTreeSet::new();
    let mut claude: BTreeMap<String, Record> = BTreeMap::new();
    let mut claude_identities = BTreeMap::new();
    let mut carry = 0;
    let mut reader = reader;
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        let n = reader
            .read_until(b'\n', &mut bytes)
            .map_err(|_| Error("read_failed"))?;
        if n == 0 {
            break;
        }
        if bytes.last() != Some(&b'\n') {
            out.coverage.pending_files = 1;
            break;
        }
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let v: Value = serde_json::from_slice(&bytes).map_err(|_| Error("invalid_jsonl"))?;
        parsed_lines += 1;
        if adapter == "codex-jsonl" && parsed_lines == 1 && old_codex_header(&v) {
            legacy_unmetered = true;
            out.coverage.unmetered_files = 1;
            thread_key = v["id"].as_str().map(|s| hash(s.as_bytes()));
            continue;
        }
        if legacy_unmetered {
            if has_usage_envelope(&v) {
                return Err(Error("unsupported_usage_record"));
            }
            let marker = v
                .as_object()
                .is_some_and(|o| o.len() == 1 && v["record_type"].is_string());
            let content = matches!(
                v["type"].as_str(),
                Some("message" | "reasoning" | "function_call" | "function_call_output")
            );
            if marker || content {
                continue;
            }
            return Err(Error("unsupported_record_shape"));
        }
        let kind = v
            .get("type")
            .and_then(Value::as_str)
            .ok_or(Error("unsupported_record_shape"))?;
        if adapter == "codex-jsonl" {
            let p = &v["payload"];
            if kind == "event_msg" && p["type"] == "task_started" {
                pending_task = p["turn_id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| hash(s.as_bytes()))
                    .filter(|id| task_ids.insert(id.clone()));
            }
            if kind == "session_meta" {
                let parent = fork_parent(p)?;
                if let Some(owner) = &fork_owner {
                    let id = p["id"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .ok_or(Error("invalid_fork_metadata"))?;
                    let key = hash(id.as_bytes());
                    if &key != owner {
                        // Only a declared ancestor's metadata may appear in an
                        // inherited prefix. It must never replace the child owner.
                        if response_usage.active() || !fork_ancestors.contains(&key) {
                            return Err(Error("unsupported_session_switch"));
                        }
                        if let Some(parent) = parent {
                            let parent_key = hash(parent.as_bytes());
                            if &parent_key == owner || parent_key == key {
                                return Err(Error("invalid_fork_metadata"));
                            }
                            record_fork_link(&mut fork_links, &key, &parent_key)?;
                            fork_ancestors.insert(parent_key);
                        }
                        continue;
                    }
                    if let Some(parent) = parent {
                        record_fork_link(&mut fork_links, owner, &hash(parent.as_bytes()))?;
                    }
                } else if let Some(parent) = parent {
                    if thread_key.is_some() || out.coverage.usage_records > 0 {
                        return Err(Error("late_fork_metadata"));
                    }
                    let id = p["id"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .ok_or(Error("invalid_fork_metadata"))?;
                    if id == parent {
                        return Err(Error("invalid_fork_metadata"));
                    }
                    let owner = hash(id.as_bytes());
                    let parent = hash(parent.as_bytes());
                    record_fork_link(&mut fork_links, &owner, &parent)?;
                    fork_creation_ms = uuid_v7_millis(id);
                    fork_owner = Some(owner);
                    fork_ancestors.insert(parent);
                }
                execution_role = if p["source"]
                    .get("subagent")
                    .is_some_and(|v| v.is_object() || v.is_string())
                {
                    Some("subagent".into())
                } else if matches!(
                    p["source"].as_str(),
                    Some("cli" | "vscode" | "exec" | "appServer")
                ) {
                    Some("main".into())
                } else {
                    None
                };
                if let Some(id) = p["id"].as_str() {
                    let next = hash(id.as_bytes());
                    if (previous.is_some() || unanchored_usage || response_usage.active())
                        && session_key != next
                    {
                        return Err(Error("unsupported_session_switch"));
                    }
                    thread_key = Some(next.clone());
                    session_key = next;
                }
            }
            if kind == "turn_context" {
                current_model = codex_model(p).map(str::to_owned);
                context_turn = p["turn_id"].as_str().map(|s| hash(s.as_bytes()));
                if fork_owner.is_some() && !response_usage.active() {
                    let own = match (
                        fork_creation_ms,
                        p["turn_id"].as_str().and_then(uuid_v7_millis),
                    ) {
                        (Some(created), Some(turn)) => turn > created,
                        _ => false,
                    };
                    if fork_own_turn && !own {
                        return Err(Error("ambiguous_fork_turn"));
                    }
                    fork_own_turn = own;
                }
            }
            if kind == "token_usage_record" {
                response_usage.record(
                    &v,
                    codex_records::Context {
                        thread: thread_key.as_deref(),
                        turn: context_turn.as_deref(),
                        model: current_model.as_deref(),
                        aliases,
                        execution_role: execution_role.as_deref(),
                    },
                    &mut out,
                )?;
                if thread_key.is_none() {
                    let key = hash(
                        p["thread_id"]
                            .as_str()
                            .ok_or(Error("missing_response_identity"))?
                            .as_bytes(),
                    );
                    session_key = key.clone();
                    thread_key = Some(key);
                }
                continue;
            }
            if kind == "compacted" && response_usage.active() {
                response_usage.compacted(&mut out.records, &mut out.coverage);
            }
            if kind != "event_msg" || p["type"] != "token_count" {
                if has_usage_envelope(&v) {
                    return Err(unsupported_usage_error(kind));
                }
                continue;
            }
            let info = &p["info"];
            if info.is_null() {
                if has_usage_envelope(&v) {
                    return Err(unsupported_usage_error(kind));
                }
                continue;
            } // rate-limit-only notification
            out.coverage.usage_records += 1;
            if fork_owner.is_some() && !response_usage.active() && !fork_own_turn {
                let mut found = false;
                for field in ["total_token_usage", "last_token_usage"] {
                    if let Some(usage) = info.get(field).filter(|u| !u.is_null()) {
                        let counters = codex_usage(usage, &mut out.coverage)?;
                        if field == "total_token_usage" {
                            previous = Some(counters);
                        }
                        found = true;
                    }
                }
                if !found {
                    return Err(Error("missing_usage"));
                }
                // Legacy snapshots in a fork prefix have no thread identity.
                // Keep the gap explicit; neither bill them to the child nor
                // claim to have proved all of them are inherited parent replay.
                out.coverage.unclassified_fork_prefix_records += 1;
                continue;
            }
            if response_usage.active() {
                response_usage.mirror(info, &mut out.coverage)?;
                continue;
            }
            if fork_owner.is_some() && fork_own_turn {
                fork_owned_legacy = true;
            }
            let total = info
                .get("total_token_usage")
                .filter(|v| !v.is_null())
                .map(|v| codex_usage(v, &mut out.coverage))
                .transpose()?;
            let last = info
                .get("last_token_usage")
                .filter(|v| !v.is_null())
                .map(|v| codex_usage(v, &mut out.coverage))
                .transpose()?;
            if let Some(u) = info.get("last_token_usage").filter(|u| !u.is_null()) {
                if u.get("input_tokens").is_none()
                    && u.get("output_tokens").is_none()
                    && u.get("total_tokens").is_some()
                {
                    return Err(Error("unsupported_last_total_only"));
                }
            }
            if total.is_some() && last.is_none() && previous.is_none() && unanchored_usage {
                return Err(Error("ambiguous_cumulative_baseline"));
            }
            // Explicit usage metadata updates context even on repeated/zero rows.
            if let Some(raw_model) = codex_model(p).or_else(|| codex_model(info)) {
                current_model = Some(raw_model.to_owned());
            }
            if total.is_none() && last.is_none() {
                return Err(Error("missing_usage"));
            }
            if let Some(t) = total {
                if info["total_token_usage"].get("total_tokens").is_some() {
                    let reported = number(&info["total_token_usage"], "total_tokens", true)?;
                    let known = t[0].checked_add(t[2]).ok_or(Error("token_overflow"))?;
                    carry = carry.max(
                        reported
                            .checked_sub(known)
                            .ok_or(Error("invalid_reported_total"))?,
                    );
                }
            }
            // Exact replay is identified before applying any baseline mutation.
            let replay_key = hash(
                &serde_json::to_vec(&(session_key.as_str(), &v["timestamp"], total, last)).unwrap(),
            );
            if !seen_codex.insert(replay_key.clone()) {
                if total.is_none() {
                    return Err(Error("ambiguous_usage_identity"));
                }
                continue;
            }
            let (increment, next) = match (total, last, previous) {
                (Some(t), _, Some(p)) if t == p => continue,
                (Some(t), Some(l), p) => {
                    if p.is_some_and(|p| delta(t, p).is_none()) {
                        // A small regression can be a stale out-of-order snapshot.
                        // Do not guess whether it represents a new billed call.
                        let sum = |a: [u64; 5]| a.into_iter().map(u128::from).sum::<u128>();
                        let before = sum(p.unwrap());
                        let now = sum(t);
                        let explicit_reset = t == l
                            && pending_task
                                .as_ref()
                                .is_some_and(|id| Some(id) == context_turn.as_ref())
                            && previous_usage_task
                                .as_ref()
                                .is_some_and(|id| Some(id) != pending_task.as_ref());
                        if !explicit_reset
                            && now > 0
                            && (now * 100 >= before * 98 || now + sum(l) * 2 >= before)
                        {
                            return Err(Error("ambiguous_counter_regression"));
                        }
                        out.coverage.counter_resets += 1;
                    }
                    (l, Some(t))
                }
                (Some(t), None, Some(p)) => (
                    delta(t, p).ok_or(Error("counter_regression_without_last"))?,
                    Some(t),
                ),
                (Some(t), None, None) => (t, Some(t)),
                (None, Some(l), Some(p)) => {
                    let mut t = p;
                    for j in 0..5 {
                        t[j] = t[j].checked_add(l[j]).ok_or(Error("token_overflow"))?;
                    }
                    (l, Some(t))
                }
                (None, Some(l), None) => (l, None),
                _ => unreachable!(),
            };
            let tokens = canonical(increment)?;
            if tokens.total()? == 0 {
                // A first known total is still an anchor when last=0. Do not
                // overwrite an existing anchor with compaction's zero snapshot.
                if previous.is_none() && !unanchored_usage {
                    previous = next;
                }
                continue;
            }
            previous = next;
            previous_usage_task = context_turn.clone();
            pending_task = None;
            unanchored_usage = previous.is_none();
            let raw_model = current_model.as_deref();
            out.records.push(Record {
                messages: MessageCount::default(),
                key: replay_key,
                model: model(raw_model, aliases, &mut out.coverage),
                model_key: hash(raw_model.unwrap_or("").as_bytes()),
                message_key: None,
                request_present: false,
                cache_write_uncertain: false,
                claude_usage_stage: 0,
                day: day(&v, &mut out.coverage),
                hour: string_event_hour(&v["timestamp"]),
                execution_role: execution_role.clone(),
                tokens,
                interval: None,
                time_basis: None,
                usage_kind: None,
                backend: None,
                provider: None,
            });
        } else {
            if kind != "assistant" {
                if has_usage_envelope(&v) {
                    return Err(unsupported_usage_error(kind));
                }
                continue;
            }
            let msg = &v["message"];
            let Some(u) = msg.get("usage").filter(|v| !v.is_null()) else {
                if has_usage_envelope(&v) {
                    return Err(unsupported_usage_error(kind));
                }
                continue;
            };
            if !u.is_object() {
                return Err(Error("unsupported_usage_shape"));
            }
            out.coverage.usage_records += 1;
            let mut tokens = Tokens {
                input: number(u, "input_tokens", true)?,
                output: number(u, "output_tokens", true)?,
                cache_read: number(u, "cache_read_input_tokens", false)?,
                cache_write: number(u, "cache_creation_input_tokens", false)?,
                reasoning: 0,
            };
            tokens.total()?;
            let mut cache_write_uncertain = false;
            if let Some(ttl) = u.get("cache_creation").filter(|t| !t.is_null()) {
                if !ttl.is_object() {
                    return Err(Error("invalid_cache_ttl"));
                }
                let sum = number(ttl, "ephemeral_5m_input_tokens", false)?
                    .checked_add(number(ttl, "ephemeral_1h_input_tokens", false)?)
                    .ok_or(Error("token_overflow"))?;
                if sum > tokens.cache_write {
                    // Contradictory parent/subset observations cannot establish
                    // cache-write usage. Keep other independent buckets and
                    // explicitly exclude this bucket from the known total.
                    cache_write_uncertain = true;
                    tokens.cache_write = 0;
                    out.coverage.invalid_cache_ttl_records += 1;
                }
                unknown_fields(
                    ttl,
                    &["ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"],
                    &mut out.coverage,
                );
            }
            unknown_fields(
                u,
                &[
                    "input_tokens",
                    "output_tokens",
                    "cache_read_input_tokens",
                    "cache_creation_input_tokens",
                    "cache_creation",
                    "service_tier",
                    "server_tool_use",
                ],
                &mut out.coverage,
            );
            let id = msg["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or(Error("missing_message_identity"))?;
            if v.get("requestId")
                .is_some_and(|v| !v.is_null() && !v.is_string())
            {
                return Err(Error("invalid_message_identity"));
            }
            let request = v["requestId"].as_str();
            if request == Some("") {
                return Err(Error("invalid_message_identity"));
            }
            let message_key = hash(id.as_bytes());
            check_claude_identity(&mut claude_identities, &message_key, request.is_some())?;
            let key = hash(&serde_json::to_vec(&(id, request)).unwrap());
            let record = Record {
                messages: MessageCount::one(),
                key: key.clone(),
                model: model(msg["model"].as_str(), aliases, &mut out.coverage),
                model_key: hash(msg["model"].as_str().unwrap_or("").as_bytes()),
                message_key: Some(message_key),
                request_present: request.is_some(),
                cache_write_uncertain,
                claude_usage_stage: if !u
                    .as_object()
                    .unwrap()
                    .contains_key("cache_read_input_tokens")
                    && !u
                        .as_object()
                        .unwrap()
                        .contains_key("cache_creation_input_tokens")
                    && tokens.output == 0
                    && msg["stop_reason"].is_null()
                {
                    1
                } else if u.get("cache_read_input_tokens").is_some()
                    && u.get("cache_creation_input_tokens").is_some()
                    && msg["stop_reason"].as_str().is_some_and(|s| !s.is_empty())
                {
                    2
                } else {
                    0
                },
                day: day(&v, &mut out.coverage),
                hour: string_event_hour(&v["timestamp"]),
                execution_role: claude_role(&v),
                tokens,
                interval: None,
                time_basis: None,
                usage_kind: None,
                backend: None,
                provider: None,
            };
            if let Some(old) = claude.get_mut(&key) {
                merge_claude(old, record, &mut out.coverage)?;
            } else {
                claude.insert(key, record);
            }
        }
    }
    if fork_owner.is_some()
        && !response_usage.active()
        && !fork_owned_legacy
        && out.coverage.unclassified_fork_prefix_records > 0
    {
        return Err(Error("unsupported_legacy_fork"));
    }
    out.codex_thread = thread_key;
    out.coverage.unknown_total_carry = carry;
    out.records.extend(claude.into_values());
    Ok(out)
}

pub fn merge_claude(old: &mut Record, mut new: Record, coverage: &mut Coverage) -> Result<()> {
    if old.model_key != new.model_key {
        return Err(Error("conflicting_message_model"));
    }
    if old.execution_role != new.execution_role {
        old.execution_role = None;
        new.execution_role = None;
    }
    if old.hour != new.hour || old.day != new.day {
        old.hour = None;
        new.hour = None;
    }
    // A provisional message can report combined input before the terminal
    // message partitions that exact total into noncached/read/write buckets.
    // Only explicit terminal metadata plus exact conservation authorizes this;
    // componentwise maxima would invent usage that never existed.
    if !old.cache_write_uncertain && !new.cache_write_uncertain {
        let partition = |r: &Record| -> Result<u64> {
            r.tokens
                .input
                .checked_add(r.tokens.cache_read)
                .and_then(|n| n.checked_add(r.tokens.cache_write))
                .ok_or(Error("token_overflow"))
        };
        if old.claude_usage_stage == 1
            && new.claude_usage_stage == 2
            && old.tokens.input == partition(&new)?
        {
            if old.day != new.day {
                new.day = None;
            }
            *old = new;
            coverage.input_reclassifications += 1;
            return Ok(());
        }
        if old.claude_usage_stage == 2
            && new.claude_usage_stage == 1
            && new.tokens.input == partition(old)?
        {
            if old.day != new.day {
                old.day = None;
            }
            coverage.input_reclassifications += 1;
            return Ok(());
        }
    }
    if old.cache_write_uncertain || new.cache_write_uncertain {
        old.cache_write_uncertain = true;
        old.tokens.cache_write = 0;
        new.tokens.cache_write = 0;
    }
    let a = old.tokens.array();
    let b = new.tokens.array();
    // Comparable updates can be reordered. Incomparable revisions are ambiguous;
    // reject rather than invent a componentwise-max bill that never existed.
    let up = a.iter().zip(b).all(|(x, y)| y >= *x);
    let down = a.iter().zip(b).all(|(x, y)| y <= *x);
    if !up && !down {
        return Err(Error("conflicting_stream_update"));
    }
    if !up {
        coverage.streaming_regressions += 1;
    }
    if up {
        old.tokens = new.tokens;
        old.claude_usage_stage = new.claude_usage_stage;
    }
    if old.day != new.day {
        old.day = None;
    }
    Ok(())
}

// Inspect only known usage envelopes, never recursively search message content
// or tool arguments for coincidental fields named usage.
fn has_usage_envelope(v: &Value) -> bool {
    ["/usage", "/data/usage", "/payload/usage", "/message/usage"]
        .iter()
        .any(|p| v.pointer(p).is_some_and(|u| !u.is_null()))
}
fn codex_model(v: &Value) -> Option<&str> {
    v.pointer("/model_info/slug")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            v.get("model")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            v.get("model_name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        })
}

pub fn check_claude_identity(
    identities: &mut BTreeMap<String, bool>,
    message: &str,
    request_present: bool,
) -> Result<()> {
    if let Some(previous) = identities.get(message) {
        if *previous != request_present {
            return Err(Error("ambiguous_message_identity"));
        }
    } else {
        identities.insert(message.to_owned(), request_present);
    }
    Ok(())
}

fn unsupported_usage_error(kind: &str) -> Error {
    if matches!(kind, "result" | "turn.completed") {
        Error("unsupported_headless_usage")
    } else {
        Error("unsupported_usage_record")
    }
}

fn fork_parent(p: &Value) -> Result<Option<&str>> {
    fn parse_id(value: Option<&Value>) -> Result<Option<&str>> {
        value
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or(Error("invalid_fork_metadata"))
            })
            .transpose()
    }
    let direct = parse_id(p.get("forked_from_id"))?;
    let nested = parse_id(p.pointer("/source/subagent/thread_spawn/parent_thread_id"))?;
    if direct.is_some() && nested.is_some() && direct != nested {
        return Err(Error("conflicting_fork_metadata"));
    }
    Ok(direct.or(nested))
}

fn record_fork_link(links: &mut BTreeMap<String, String>, child: &str, parent: &str) -> Result<()> {
    if links.get(child).is_some_and(|previous| previous != parent) {
        return Err(Error("conflicting_fork_metadata"));
    }
    let mut ancestor = parent;
    loop {
        if ancestor == child {
            return Err(Error("invalid_fork_metadata"));
        }
        match links.get(ancestor) {
            Some(next) => ancestor = next,
            None => break,
        }
    }
    links.insert(child.to_owned(), parent.to_owned());
    Ok(())
}

// Only the timestamp portion is ordered; UUID random tails never establish
// causality. Equal-millisecond and older/non-v7 turns remain unclassified.
fn uuid_v7_millis(id: &str) -> Option<u64> {
    let bytes = id.as_bytes();
    if bytes.len() != 36 || bytes[14] != b'7' || !b"89aAbB".contains(&bytes[19]) {
        return None;
    }
    for (i, b) in bytes.iter().enumerate() {
        if [8, 13, 18, 23].contains(&i) {
            if *b != b'-' {
                return None;
            }
        } else if !b.is_ascii_hexdigit() {
            return None;
        }
    }
    u64::from_str_radix(&format!("{}{}", &id[..8], &id[9..13]), 16).ok()
}
fn old_codex_header(v: &Value) -> bool {
    let Some(o) = v.as_object() else {
        return false;
    };
    o.keys()
        .all(|k| ["id", "timestamp", "instructions", "git"].contains(&k.as_str()))
        && v["id"].as_str().is_some_and(|s| !s.is_empty())
        && v["timestamp"]
            .as_str()
            .is_some_and(|s| DateTime::parse_from_rfc3339(s).is_ok())
        && v.get("instructions").is_some()
}
