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
            if target.starts_with(root) {
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
    let mut agents: BTreeMap<String, Tokens> = BTreeMap::new();
    let mut models: BTreeMap<String, Tokens> = BTreeMap::new();
    let mut days = std::collections::BTreeSet::new();
    let mut total = Tokens::default();
    let mut unallocated = Tokens::default();
    for source in &snapshot.sources {
        if !label(&source.source_id)
            || !["ok", "empty", "partial", "unmetered"].contains(&source.status.as_str())
        {
            return Err(Error("invalid_snapshot"));
        }
    }
    for row in &snapshot.rows {
        if !["codex", "claude"].contains(&row.agent.as_str())
            || !label(&row.model)
            || !snapshot
                .sources
                .iter()
                .any(|s| s.source_id == row.source_id)
        {
            return Err(Error("invalid_snapshot"));
        }
        if let Some(day) = &row.day {
            if row.time_basis != "event_timestamp"
                || chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_err()
            {
                return Err(Error("invalid_snapshot"));
            }
            days.insert(day.clone());
        } else {
            if row.time_basis != "unallocated" {
                return Err(Error("invalid_snapshot"));
            }
            unallocated = unallocated.checked_add(row.tokens)?;
        }
        total = total.checked_add(row.tokens)?;
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
        "unallocated_tokens":unallocated.total()?,"first_day":days.first(),"last_day":days.last(),"days":days.len(),
        "by_agent":numeric(agents)?,"by_model":numeric(models)?,"sources":snapshot.sources,"cost":null}),
    )
}
