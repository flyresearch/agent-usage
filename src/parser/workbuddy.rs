//! WorkBuddy normalized per-call usage, independently implemented from observed
//! field structure. No database, credentials, trace totals, or billing calls.
use super::*;

fn detail(u: &Value, field: &str, key: &str, cov: &mut Coverage) -> Result<u64> {
    let value = u.get(field).ok_or(Error("missing_token_field"))?;
    let entries = value.as_array().ok_or(Error("unsupported_usage_shape"))?;
    entries.iter().try_fold(0u64, |sum, entry| {
        unknown_fields(entry, &[key], cov);
        sum.checked_add(number(entry, key, true)?)
            .ok_or(Error("token_overflow"))
    })
}

pub(super) fn parse(
    mut reader: impl BufRead,
    aliases: &BTreeMap<String, String>,
) -> Result<Parsed> {
    let mut out = Parsed::default();
    let mut bytes = Vec::new();
    let mut seen = BTreeMap::new();
    let mut unmetered = BTreeSet::new();
    let mut anonymous_unmetered = 0u64;
    loop {
        bytes.clear();
        if reader
            .read_until(b'\n', &mut bytes)
            .map_err(|_| Error("read_failed"))?
            == 0
        {
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
        let kind = v["type"]
            .as_str()
            .ok_or(Error("unsupported_record_shape"))?;
        let pd = &v["providerData"];
        let Some(u) = pd.get("usage") else {
            // A mirror without its normalized authority must not silently vanish.
            if v["message"].get("usage").is_some() || pd.get("rawUsage").is_some() {
                return Err(Error("unsupported_usage_record"));
            }
            if !matches!(
                kind,
                "message"
                    | "function_call"
                    | "function_call_result"
                    | "reasoning"
                    | "file-history-snapshot"
                    | "session-meta"
                    | "ai-title"
            ) {
                return Err(Error("unsupported_record_shape"));
            }
            // Streaming/tool-call fragments can precede the metered record.
            // Reconcile by the same response identity after reading the file.
            let possible_response = kind == "function_call"
                || (kind == "message"
                    && !matches!(
                        v["role"].as_str(),
                        Some("user" | "system" | "developer" | "tool")
                    ));
            if possible_response {
                if let Some(id) = pd["messageId"].as_str().filter(|s| !s.is_empty()) {
                    unmetered.insert(hash(&serde_json::to_vec(&("workbuddy", id)).unwrap()));
                } else {
                    anonymous_unmetered = anonymous_unmetered
                        .checked_add(1)
                        .ok_or(Error("token_overflow"))?;
                }
            }
            continue;
        };
        if !matches!(kind, "message" | "function_call") || !u.is_object() {
            return Err(Error("unsupported_usage_record"));
        }
        out.coverage.usage_records += 1;
        unknown_fields(
            u,
            &[
                "requests",
                "inputTokens",
                "outputTokens",
                "totalTokens",
                "inputTokensDetails",
                "outputTokensDetails",
            ],
            &mut out.coverage,
        );
        if number(u, "requests", true)? != 1 {
            return Err(Error("unsupported_request_count"));
        }
        let input = number(u, "inputTokens", true)?;
        let output = number(u, "outputTokens", true)?;
        let total = input.checked_add(output).ok_or(Error("token_overflow"))?;
        if number(u, "totalTokens", true)? != total {
            return Err(Error("invalid_reported_total"));
        }
        let cached = detail(u, "inputTokensDetails", "cached_tokens", &mut out.coverage)?;
        let reasoning = detail(
            u,
            "outputTokensDetails",
            "reasoning_tokens",
            &mut out.coverage,
        )?;
        // Positive cache-write semantics are not yet qualified. Fail closed rather
        // than invent a bucket mapping or count a disputed component as zero.
        let raw = &pd["rawUsage"];
        if !raw.is_null() && !raw.is_object() {
            return Err(Error("unsupported_usage_shape"));
        }
        let mut write_known = false;
        for field in ["cache_creation_input_tokens", "prompt_cache_write_tokens"] {
            if raw.get(field).is_some() {
                write_known = true;
                if number(raw, field, true)? != 0 {
                    return Err(Error("unsupported_cache_write"));
                }
            }
        }
        if !write_known {
            return Err(Error("unsupported_cache_write"));
        }
        for (field, expected) in [
            ("prompt_tokens", input),
            ("completion_tokens", output),
            ("total_tokens", total),
            ("prompt_cache_hit_tokens", cached),
        ] {
            if raw.get(field).is_some() && number(raw, field, true)? != expected {
                return Err(Error("conflicting_usage_fields"));
            }
        }
        let tokens = canonical([input, cached, output, reasoning, 0])?;
        let id = pd["messageId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(Error("missing_message_identity"))?;
        let key = hash(&serde_json::to_vec(&("workbuddy", id)).unwrap());
        let date = v["timestamp"]
            .as_i64()
            .and_then(DateTime::from_timestamp_millis)
            .map(|d| {
                d.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap())
                    .format("%Y-%m-%d")
                    .to_string()
            });
        if date.is_none() {
            out.coverage.missing_time_records += 1;
        }
        let raw_model = pd["model"].as_str().filter(|s| !s.is_empty());
        let record = Record {
            messages: MessageCount::one(),
            key: key.clone(),
            model: model(raw_model, aliases, &mut out.coverage),
            model_key: hash(raw_model.unwrap_or("").as_bytes()),
            message_key: None,
            request_present: false,
            cache_write_uncertain: false,
            claude_usage_stage: 0,
            day: date,
            hour: event_hour(&v["timestamp"]),
            execution_role: None,
            tokens,
            interval: None,
            time_basis: None,
            usage_kind: None,
            backend: None,
            provider: None,
        };
        // Dedup across files is also enforced by collector, including warm hits.
        if let Some(old) = seen.get(&key) {
            let old: &Record = old;
            if old.tokens != record.tokens
                || old.day != record.day
                || old.hour != record.hour
                || old.model_key != record.model_key
            {
                return Err(Error("conflicting_replay"));
            }
        } else {
            seen.insert(key, record);
        }
    }
    out.coverage.unclassified_usage_records = anonymous_unmetered
        .checked_add(
            unmetered
                .iter()
                .filter(|key| !seen.contains_key(*key))
                .count() as u64,
        )
        .ok_or(Error("token_overflow"))?;
    if seen.is_empty() && out.coverage.unclassified_usage_records > 0 {
        out.coverage.unmetered_files = 1;
    }
    out.records = seen.into_values().collect();
    Ok(out)
}
