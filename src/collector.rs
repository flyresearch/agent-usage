use crate::{
    parser::{self, Parsed, Record},
    protocol::*,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    len: u64,
    modified: u128,
    identity: String,
}
fn stamp(path: &Path) -> Result<Stamp> {
    let m = fs::metadata(path).map_err(|_| Error("metadata_failed"))?;
    let modified = m
        .modified()
        .map_err(|_| Error("metadata_failed"))?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error("invalid_file_time"))?
        .as_nanos();
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}:{}:{}", m.dev(), m.ino(), m.ctime(), m.ctime_nsec())
    };
    #[cfg(not(unix))]
    let identity = format!("{:?}", m.created().ok());
    Ok(Stamp {
        len: m.len(),
        modified,
        identity,
    })
}
#[derive(Serialize, Deserialize)]
struct Cached {
    stamp: Stamp,
    parsed: Parsed,
}
#[derive(Default, Serialize, Deserialize)]
struct Cache {
    version: String,
    fingerprint: String,
    files: BTreeMap<String, Cached>,
}

fn validate(config: &Config, active_source: &mut Option<String>) -> Result<Vec<(usize, PathBuf)>> {
    if config.schema_version != 1 {
        return Err(Error("unsupported_config_version"));
    }
    if !label(&config.host_id)
        || config.sources.is_empty()
        || config.model_aliases.values().any(|s| !label(s))
    {
        return Err(Error("invalid_config"));
    }
    let mut ids = BTreeSet::new();
    let mut roots: Vec<(usize, PathBuf)> = Vec::new();
    for (i, s) in config.sources.iter().enumerate() {
        *active_source = label(&s.id).then(|| s.id.clone());
        if !label(&s.id) || !ids.insert(&s.id) || s.paths.is_empty() {
            return Err(Error("invalid_source_config"));
        }
        if ![
            "codex-jsonl",
            "claude-jsonl",
            "agy-sqlite",
            "hermes-sqlite",
            "openclaw-sqlite",
            "openclaw-codex-jsonl",
        ]
        .contains(&s.adapter.as_str())
        {
            return Err(Error("unsupported_adapter"));
        }
        for path in &s.paths {
            if !path.is_absolute() {
                return Err(Error("path_must_be_absolute"));
            }
            let p = fs::canonicalize(path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Error("source_missing")
                } else {
                    Error("source_unreadable")
                }
            })?;
            if ["hermes-sqlite", "openclaw-sqlite"].contains(&s.adapter.as_str()) && !p.is_file() {
                return Err(Error("source_must_be_database_file"));
            }
            if !(p.is_dir() || s.adapter.ends_with("-sqlite") && p.is_file()) {
                return Err(Error("source_must_be_directory"));
            }
            if roots
                .iter()
                .any(|(_, r)| p.starts_with(r) || r.starts_with(&p))
            {
                return Err(Error("overlapping_paths"));
            }
            roots.push((i, p));
        }
    }
    *active_source = None;
    Ok(roots)
}

/// One-shot, all-or-nothing host collection. Cache contains only allowed usage
/// projection plus hashed identities; a failed scan never publishes a snapshot.
/// Library callers sharing state must hold `locking::CollectionLocks` through
/// collection and publication; the CLI does this automatically.
pub fn collect(config: &Config, cache_path: Option<&Path>) -> Result<Snapshot> {
    collect_detailed(config, cache_path).map_err(|failure| failure.error)
}
pub fn validate_sources(config: &Config) -> std::result::Result<(), CollectionFailure> {
    let mut active_source = None;
    validate(config, &mut active_source)
        .map(|_| ())
        .map_err(|error| CollectionFailure {
            error,
            source_id: active_source,
        })
}
pub fn collect_detailed(
    config: &Config,
    cache_path: Option<&Path>,
) -> std::result::Result<Snapshot, CollectionFailure> {
    let mut active_source = None;
    collect_inner(config, cache_path, &mut active_source).map_err(|error| CollectionFailure {
        error,
        source_id: active_source,
    })
}
fn collect_inner(
    config: &Config,
    cache_path: Option<&Path>,
    active_source: &mut Option<String>,
) -> Result<Snapshot> {
    let roots = validate(config, active_source)?;
    // Resolve bare filenames before checking the parent or creating temp files.
    let cache_path = cache_path
        .map(|p| {
            if p.is_absolute() {
                Ok(p.to_path_buf())
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(p))
                    .map_err(|_| Error("invalid_cache_path"))
            }
        })
        .transpose()?;
    let cache_path = cache_path.as_deref();
    if let Some(path) = cache_path {
        let parent = path.parent().ok_or(Error("invalid_cache_path"))?;
        let canonical = fs::canonicalize(parent).map_err(|_| Error("cache_parent_missing"))?;
        let target = canonical.join(path.file_name().ok_or(Error("invalid_cache_path"))?);
        if roots
            .iter()
            .any(|(_, r)| crate::output::source_contains(r, &target))
        {
            return Err(Error("cache_inside_source"));
        }
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(Error("cache_symlink"));
        }
    }
    let fingerprint = hash(&serde_json::to_vec(config).map_err(|_| Error("invalid_config"))?);
    let mut cache = match cache_path {
        Some(p) => match File::open(p) {
            Ok(f) => {
                serde_json::from_reader(BufReader::new(f)).map_err(|_| Error("invalid_cache"))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Cache::default(),
            Err(_) => return Err(Error("cache_read_failed")),
        },
        None => Cache::default(),
    };
    let mut cache_dirty = cache.version != PARSER_VERSION || cache.fingerprint != fingerprint;
    if cache_dirty {
        cache = Cache::default();
    }
    let mut next = Cache {
        version: PARSER_VERSION.into(),
        fingerprint: fingerprint.clone(),
        ..Cache::default()
    };
    let mut statuses: Vec<SourceStatus> = config
        .sources
        .iter()
        .map(|s| SourceStatus {
            source_id: s.id.clone(),
            status: "ok".into(),
            files: 0,
            cache_hits: 0,
            bytes_read: 0,
            coverage: Coverage::default(),
        })
        .collect();
    let mut paths = Vec::new();
    let mut physical = HashSet::new();
    for (i, root) in &roots {
        *active_source = Some(config.sources[*i].id.clone());
        let walker = WalkDir::new(root).follow_links(false);
        let walker = if config.sources[*i].adapter == "agy-sqlite" {
            walker.max_depth(1)
        } else {
            walker
        };
        for entry in walker {
            let entry = entry.map_err(|_| Error("source_unreadable"))?;
            // Claude can alias a subagent log inside the same configured root.
            // Its canonical regular JSONL target is already enumerated by this
            // walk, so skip the alias without following it. Directories, dangling
            // links, other formats and out-of-scope targets still fail closed.
            if entry.file_type().is_symlink() {
                let target = fs::canonicalize(entry.path()).map_err(|_| Error("nested_symlink"))?;
                if !config.sources[*i].adapter.ends_with("-sqlite")
                    && target.starts_with(root)
                    && target.is_file()
                    && target.extension().is_some_and(|e| e == "jsonl")
                    && entry.path().extension().is_some_and(|e| e == "jsonl")
                {
                    statuses[*i].coverage.deduplicated_symlink_files += 1;
                    continue;
                }
                return Err(Error("nested_symlink"));
            }
            let database = config.sources[*i].adapter.ends_with("-sqlite");
            let selected = if database {
                root.is_file()
                    || entry
                        .path()
                        .extension()
                        .is_some_and(|e| e == "db" || e == "sqlite")
            } else {
                entry.path().extension().is_some_and(|e| e == "jsonl")
            };
            if !entry.file_type().is_file() || !selected {
                continue;
            }
            #[cfg(unix)]
            let handle = {
                use std::os::unix::fs::MetadataExt;
                let m = fs::metadata(entry.path()).map_err(|_| Error("source_unreadable"))?;
                (m.dev(), m.ino())
            };
            #[cfg(not(unix))]
            let handle = {
                if physical.len() >= 4096 {
                    return Err(Error("platform_file_limit"));
                }
                same_file::Handle::from_path(entry.path())
                    .map_err(|_| Error("source_unreadable"))?
            };
            if !physical.insert(handle) {
                return Err(Error("duplicate_physical_file"));
            }
            paths.push((*i, entry.into_path()));
        }
    }
    *active_source = None;
    // Dedicated backend logs establish the thread set before SQLite mirror classification.
    paths.sort_by_key(|(i, p)| {
        (
            config.sources[*i].adapter.ends_with("-sqlite"),
            *i,
            p.clone(),
        )
    });
    let mut backend_threads = BTreeSet::new();
    let mut records: BTreeMap<(usize, String), Record> = BTreeMap::new();
    let mut codex_modes: BTreeMap<(usize, String), bool> = BTreeMap::new();
    let mut claude_identities: Vec<BTreeMap<String, bool>> =
        vec![BTreeMap::new(); config.sources.len()];
    for (i, path) in paths {
        *active_source = Some(config.sources[i].id.clone());
        let key = hash(path.as_os_str().as_encoded_bytes());
        let before = stamp(&path)?;
        let status = &mut statuses[i];
        status.files += 1;
        let database = config.sources[i].adapter.ends_with("-sqlite");
        let parsed = if database {
            // Always query a readonly transaction: the main file's metadata
            // cannot represent committed WAL updates.
            let (parsed, bytes) = match config.sources[i].adapter.as_str() {
                "agy-sqlite" => crate::sqlite::agy::parse_file(&path, &config.model_aliases, &key)?,
                "hermes-sqlite" => {
                    crate::sqlite::hermes::parse_file(&path, &config.model_aliases, &key)?
                }
                "openclaw-sqlite" => crate::sqlite::openclaw::parse_file(
                    &path,
                    &config.model_aliases,
                    &key,
                    &backend_threads,
                )?,
                _ => unreachable!(),
            };
            status.bytes_read += bytes;
            parsed
        } else if let Some(old) = cache.files.remove(&key).filter(|c| c.stamp == before) {
            status.cache_hits += 1;
            old.parsed
        } else {
            cache_dirty = true;
            let f = File::open(&path).map_err(|_| Error("source_unreadable"))?;
            // Limit to size observed at scan start. A concurrent append is left
            // for the next scan; mutation during reading is retried as a failure.
            let result = parser::parse(
                BufReader::new(f.take(before.len)),
                if config.sources[i].adapter == "openclaw-codex-jsonl" {
                    "codex-jsonl"
                } else {
                    &config.sources[i].adapter
                },
                &config.model_aliases,
                &key,
            )?;
            status.bytes_read += before.len;
            result
        };
        if !database && stamp(&path)? != before {
            return Err(Error("source_changed_during_scan"));
        }
        if config.sources[i].adapter == "openclaw-codex-jsonl" {
            if let Some(thread) = &parsed.codex_thread {
                backend_threads.insert(thread.clone());
            }
        }
        if !parsed.records.is_empty() {
            if let Some(thread) = &parsed.codex_thread {
                let modern = parsed.coverage.response_usage_records > 0;
                if let Some(previous) = codex_modes.insert((i, thread.clone()), modern) {
                    if previous != modern {
                        return Err(Error("mixed_codex_storage"));
                    }
                }
            }
        }
        add_coverage(&mut status.coverage, &parsed.coverage)?;
        for original in &parsed.records {
            let mut attributed = original.clone();
            if config.sources[i].adapter == "openclaw-codex-jsonl" {
                attributed.backend = Some("codex".into());
            }
            let r = &attributed;
            if config.sources[i].adapter == "claude-jsonl" {
                let message = r
                    .message_key
                    .as_deref()
                    .ok_or(Error("invalid_cache_identity"))?;
                parser::check_claude_identity(
                    &mut claude_identities[i],
                    message,
                    r.request_present,
                )?;
            }
            let rk = (i, r.key.clone());
            if let Some(old) = records.get_mut(&rk) {
                if config.sources[i].adapter == "claude-jsonl" {
                    parser::merge_claude(old, r.clone(), &mut status.coverage)?;
                } else if old.messages != r.messages
                    || old.tokens != r.tokens
                    || old.day != r.day
                    || old.model_key != r.model_key
                    || old.backend != r.backend
                    || old.provider != r.provider
                    || old.time_basis != r.time_basis
                    || old.interval != r.interval
                    || old.usage_kind != r.usage_kind
                {
                    return Err(Error("conflicting_replay"));
                }
            } else {
                records.insert(rk, r.clone());
            }
        }
        if !database {
            next.files.insert(
                key,
                Cached {
                    stamp: before,
                    parsed,
                },
            );
        }
    }
    *active_source = None;
    type GroupKey = (
        usize,
        String,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let mut grouped: BTreeMap<GroupKey, (Tokens, Option<UsageInterval>, MessageCount)> =
        BTreeMap::new();
    let mut totals = Tokens::default();
    for ((i, _), r) in records {
        if config.sources[i].adapter != "hermes-sqlite"
            && r.messages != MessageCount::default()
            && r.messages != MessageCount::one()
        {
            return Err(Error("invalid_cache_projection"));
        }
        *active_source = Some(config.sources[i].id.clone());
        if r.claude_usage_stage == 1 {
            statuses[i].coverage.unclassified_usage_records += 1;
            continue;
        }
        if r.day.is_none() {
            statuses[i].coverage.unallocated_tokens = statuses[i]
                .coverage
                .unallocated_tokens
                .checked_add(r.tokens.total()?)
                .ok_or(Error("token_overflow"))?;
        }
        totals = totals.checked_add(r.tokens)?;
        let basis = r.time_basis.unwrap_or_else(|| {
            if r.day.is_some() {
                "event_timestamp".into()
            } else {
                "unallocated".into()
            }
        });
        let key = (
            i,
            r.model,
            r.day,
            basis,
            r.usage_kind,
            r.backend,
            r.provider,
        );
        if let Some((tokens, bounds, messages)) = grouped.get_mut(&key) {
            *tokens = tokens.checked_add(r.tokens)?;
            *messages = messages.checked_add(r.messages)?;
            if let (Some(a), Some(b)) = (bounds.as_mut(), r.interval.as_ref()) {
                a.merge(b);
            } else {
                *bounds = None;
            }
        } else {
            grouped.insert(key, (r.tokens, r.interval, r.messages));
        }
    }
    *active_source = None;
    for s in &mut statuses {
        if s.files == 0 {
            s.status = "empty".into();
        } else if s.coverage.unmetered_files == s.files {
            s.status = "unmetered".into();
        } else if s.coverage.unclassified_usage_records > 0
            || s.coverage.unmetered_files > 0
            || s.coverage.invalid_cache_ttl_records > 0
            || s.coverage.unclassified_fork_prefix_records > 0
            || s.coverage.negative_residual_records > 0
            || s.coverage.orphan_usage_records > 0
            || s.coverage.missing_backend_threads > 0
        {
            s.status = "partial".into();
        }
    }
    // Merging open intervals can leave neither bound known. Normalize that
    // bucket to unallocated, then coalesce with any already unallocated rows.
    let mut normalized: BTreeMap<GroupKey, (Tokens, Option<UsageInterval>, MessageCount)> =
        BTreeMap::new();
    for (mut key, (tokens, mut bounds, messages)) in grouped {
        if bounds
            .as_ref()
            .is_some_and(|b| b.start.is_none() && b.end.is_none())
        {
            bounds = None;
            key.3 = "unallocated".into();
        }
        if let Some((old, _, old_messages)) = normalized.get_mut(&key) {
            *old = old.checked_add(tokens)?;
            *old_messages = old_messages.checked_add(messages)?;
        } else {
            normalized.insert(key, (tokens, bounds, messages));
        }
    }
    let rows = normalized
        .into_iter()
        .map(
            |(
                (i, model, day, time_basis, usage_kind, backend, provider),
                (tokens, interval, messages),
            )| {
                Row {
                    source_id: config.sources[i].id.clone(),
                    agent: match config.sources[i].adapter.as_str() {
                        "codex-jsonl" => "codex",
                        "claude-jsonl" => "claude",
                        "agy-sqlite" => "agy",
                        "hermes-sqlite" => "hermes",
                        "openclaw-sqlite" | "openclaw-codex-jsonl" => "openclaw",
                        _ => unreachable!(),
                    }
                    .into(),
                    model,
                    day,
                    time_basis,
                    usage_kind,
                    backend,
                    provider,
                    interval,
                    tokens,
                    messages,
                }
            },
        )
        .collect();
    let coverage_complete = statuses.iter().all(|s| {
        let c = &s.coverage;
        c.missing_backend_threads == 0
            && c.negative_residual_records == 0
            && c.orphan_usage_records == 0
            && c.unclassified_usage_records == 0
            && c.missing_identity_records == 0
            && c.ambiguous_time_records == 0
            && c.invalid_cache_ttl_records == 0
            && c.unmetered_files == 0
            && c.unclassified_fork_prefix_records == 0
            && c.pending_files == 0
            && c.unknown_total_carry == 0
            && c.missing_time_records == 0
            && c.unknown_model_records == 0
            && c.unsupported_fields == 0
            && c.counter_resets == 0
            && c.unallocated_tokens == 0
    });
    let snapshot = Snapshot {
        schema_version: 1,
        collector_version: env!("CARGO_PKG_VERSION").into(),
        parser_version: PARSER_VERSION.into(),
        host_id: config.host_id.clone(),
        configuration_fingerprint: fingerprint,
        generated_at: Utc::now().to_rfc3339(),
        timezone: "Asia/Singapore".into(),
        complete: true,
        coverage_complete,
        sources: statuses,
        rows,
        totals,
        cost: None,
        sqlite_version: config
            .sources
            .iter()
            .any(|s| s.adapter.ends_with("-sqlite"))
            .then(|| rusqlite::version().to_owned()),
    };
    // Cached projections pass the same outbound contract as freshly parsed
    // records. Never publish arbitrary cached text as a model/date/backend.
    if snapshot.rows.iter().any(|row| {
        row.model != "unknown"
            && !config
                .model_aliases
                .values()
                .any(|model| model == &row.model)
    }) {
        return Err(Error("invalid_cache_projection"));
    }
    crate::output::summary(&snapshot)?;
    cache_dirty |= !cache.files.is_empty();
    if let Some(p) = cache_path.filter(|_| cache_dirty) {
        let mut temp = tempfile::NamedTempFile::new_in(p.parent().unwrap())
            .map_err(|_| Error("cache_write_failed"))?;
        {
            let mut writer = BufWriter::with_capacity(128 * 1024, temp.as_file_mut());
            serde_json::to_writer(&mut writer, &next).map_err(|_| Error("cache_write_failed"))?;
            writer.flush().map_err(|_| Error("cache_write_failed"))?;
        }
        temp.as_file()
            .sync_all()
            .map_err(|_| Error("cache_write_failed"))?;
        temp.persist(p).map_err(|_| Error("cache_write_failed"))?;
    }
    Ok(snapshot)
}
fn add_coverage(a: &mut Coverage, b: &Coverage) -> Result<()> {
    macro_rules! add { ($($f:ident),*) => {$ (a.$f=a.$f.checked_add(b.$f).ok_or(Error("coverage_overflow"))?;)*}; }
    add!(
        usage_records,
        deduplicated_symlink_files,
        input_reclassifications,
        runtime_usage_records,
        missing_backend_threads,
        auxiliary_usage_records,
        residual_usage_records,
        negative_residual_records,
        orphan_usage_records,
        unclassified_usage_records,
        missing_identity_records,
        ambiguous_time_records,
        invalid_cache_ttl_records,
        unmetered_files,
        unclassified_fork_prefix_records,
        response_usage_records,
        mirrored_usage_records,
        context_estimate_records,
        pending_files,
        unknown_total_carry,
        missing_time_records,
        unknown_model_records,
        unsupported_fields,
        counter_resets,
        streaming_regressions,
        unallocated_tokens
    );
    Ok(())
}
