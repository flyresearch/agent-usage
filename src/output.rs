//! Local snapshot publication and numeric report. No network or source discovery.
use crate::protocol::*;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

pub fn destination(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| Error("invalid_output_path"))?
            .join(path)
    };
    let parent = fs::canonicalize(absolute.parent().ok_or(Error("invalid_output_path"))?)
        .map_err(|_| Error("output_parent_missing"))?;
    let result = parent.join(absolute.file_name().ok_or(Error("invalid_output_path"))?);
    if fs::symlink_metadata(&result).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(Error("output_symlink"));
    }
    Ok(result)
}

// A configured SQLite file also owns its journal sidecars. State destinations
// must not overwrite them merely because they are siblings of the main file.
pub fn source_contains(root: &Path, target: &Path) -> bool {
    target.starts_with(root)
        || (!root.is_dir()
            && ["-wal", "-shm", "-journal"].iter().any(|suffix| {
                let mut sidecar = root.as_os_str().to_os_string();
                sidecar.push(suffix);
                target == Path::new(&sidecar)
            }))
}

pub fn validate_destination(
    path: &Path,
    config_path: &Path,
    cache: Option<&Path>,
    config: &Config,
) -> Result<PathBuf> {
    let target = destination(path)?;
    let config_path = fs::canonicalize(config_path).map_err(|_| Error("config_read_failed"))?;
    if target == config_path
        || cache
            .map(destination)
            .transpose()?
            .is_some_and(|c| c == target)
    {
        return Err(Error("output_path_conflict"));
    }
    for source in &config.sources {
        for root in &source.paths {
            let root = fs::canonicalize(root).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Error("source_missing")
                } else {
                    Error("source_unreadable")
                }
            })?;
            if source_contains(&root, &target) {
                return Err(Error("output_inside_source"));
            }
        }
    }
    Ok(target)
}

pub fn publish(path: &Path, snapshot: &Snapshot) -> Result<()> {
    if !snapshot.complete {
        return Err(Error("incomplete_snapshot"));
    }
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().ok_or(Error("invalid_output_path"))?)
            .map_err(|_| Error("output_write_failed"))?;
    {
        let mut writer = BufWriter::with_capacity(128 * 1024, temp.as_file_mut());
        serde_json::to_writer(&mut writer, snapshot).map_err(|_| Error("output_write_failed"))?;
        writer
            .write_all(b"\n")
            .map_err(|_| Error("output_write_failed"))?;
        writer.flush().map_err(|_| Error("output_write_failed"))?;
    }
    temp.as_file()
        .sync_all()
        .map_err(|_| Error("output_write_failed"))?;
    temp.persist(path)
        .map_err(|_| Error("output_write_failed"))?;
    Ok(())
}

pub fn summary(snapshot: &Snapshot) -> Result<Value> {
    if snapshot.schema_version != 1 || !snapshot.complete || !label(&snapshot.host_id) {
        return Err(Error("invalid_snapshot"));
    }
    if chrono::DateTime::parse_from_rfc3339(&snapshot.generated_at).is_err()
        || snapshot.timezone != "Asia/Singapore"
    {
        return Err(Error("invalid_snapshot"));
    }
    if snapshot.sqlite_version.as_ref().is_some_and(|v| {
        v.is_empty() || v.len() > 32 || !v.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    }) {
        return Err(Error("invalid_snapshot"));
    }
    let mut agents: BTreeMap<String, Tokens> = BTreeMap::new();
    let mut models: BTreeMap<String, Tokens> = BTreeMap::new();
    let mut days = std::collections::BTreeSet::new();
    let mut total = Tokens::default();
    let mut messages = MessageCount {
        known: 0,
        complete: true,
    };
    let mut unallocated = Tokens::default();
    for source in &snapshot.sources {
        if !label(&source.source_id)
            || !["ok", "empty", "partial", "unmetered"].contains(&source.status.as_str())
        {
            return Err(Error("invalid_snapshot"));
        }
    }
    for row in &snapshot.rows {
        if !["codex", "claude", "agy", "hermes", "openclaw", "workbuddy"]
            .contains(&row.agent.as_str())
            || !label(&row.model)
            || !snapshot
                .sources
                .iter()
                .any(|s| s.source_id == row.source_id)
        {
            return Err(Error("invalid_snapshot"));
        }
        if row
            .usage_kind
            .as_deref()
            .is_some_and(|v| !["main", "auxiliary", "residual"].contains(&v))
            || row.backend.as_deref().is_some_and(|v| !label(v))
            || row.provider.as_deref().is_some_and(|v| !label(v))
        {
            return Err(Error("invalid_snapshot"));
        }
        let interval_basis =
            ["aggregate_interval", "unallocated_interval"].contains(&row.time_basis.as_str());
        if interval_basis != row.interval.is_some() {
            return Err(Error("invalid_snapshot"));
        }
        if let Some(bounds) = &row.interval {
            let parse =
                |v: &Option<String>| -> Result<Option<chrono::DateTime<chrono::FixedOffset>>> {
                    v.as_ref()
                        .map(|v| {
                            chrono::DateTime::parse_from_rfc3339(v)
                                .map_err(|_| Error("invalid_snapshot"))
                        })
                        .transpose()
                };
            let start = parse(&bounds.start)?;
            let end = parse(&bounds.end)?;
            if (start.is_none() && end.is_none()) || matches!((start,end),(Some(a),Some(b)) if a>b)
            {
                return Err(Error("invalid_snapshot"));
            }
            if let Some(day) = &row.day {
                let zone = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
                if !matches!((start,end),(Some(a),Some(b)) if a.with_timezone(&zone).format("%Y-%m-%d").to_string()==*day && b.with_timezone(&zone).format("%Y-%m-%d").to_string()==*day)
                {
                    return Err(Error("invalid_snapshot"));
                }
            }
        }
        if let Some(day) = &row.day {
            if !["event_timestamp", "aggregate_interval"].contains(&row.time_basis.as_str())
                || chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_err()
            {
                return Err(Error("invalid_snapshot"));
            }
            days.insert(day.clone());
        } else {
            if !["unallocated", "unallocated_interval"].contains(&row.time_basis.as_str()) {
                return Err(Error("invalid_snapshot"));
            }
            unallocated = unallocated.checked_add(row.tokens)?;
        }
        total = total.checked_add(row.tokens)?;
        messages = messages.checked_add(row.messages)?;
        let agent = agents.entry(row.agent.clone()).or_default();
        *agent = agent.checked_add(row.tokens)?;
        let model = models.entry(row.model.clone()).or_default();
        *model = model.checked_add(row.tokens)?;
    }
    if total != snapshot.totals {
        return Err(Error("snapshot_total_mismatch"));
    }
    let numeric = |groups: BTreeMap<String, Tokens>| -> Result<BTreeMap<String, Value>> {
        groups
            .into_iter()
            .map(|(key, t)| Ok((key, json!({"tokens":t,"total_tokens":t.total()?}))))
            .collect()
    };
    Ok(
        json!({"schema_version":1,"kind":"usage_summary","host_id":snapshot.host_id,"generated_at":snapshot.generated_at,
        "complete":true,"coverage_complete":snapshot.coverage_complete,"known_tokens":total,"known_total_tokens":total.total()?,
        "messages":messages,"unallocated_tokens":unallocated.total()?,"first_day":days.first(),"last_day":days.last(),"days":days.len(),
        "by_agent":numeric(agents)?,"by_model":numeric(models)?,"sources":snapshot.sources,"cost":null,"sqlite_version":snapshot.sqlite_version}),
    )
}
