//! On-demand host snapshots and transactional last-good storage.
pub mod store;
pub mod transport;
use crate::{output, protocol::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetConfig {
    pub schema_version: u32,
    pub hosts: Vec<HostConfig>,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "default_stale")]
    pub stale_after_seconds: u64,
}
fn default_timeout() -> u64 {
    120
}
fn default_stale() -> u64 {
    900
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    pub host_id: String,
    pub configuration_fingerprint: String,
    pub transport: Transport,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transport {
    Local {
        executable: String,
        config: String,
        cache: Option<String>,
    },
    Ssh {
        destination: String,
        executable: String,
        config: String,
        cache: Option<String>,
        shell: RemoteShell,
    },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteShell {
    Posix,
    Powershell,
}
fn fingerprint(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
impl FleetConfig {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.hosts.is_empty()
            || self.hosts.len() > 64
            || !(1..=3600).contains(&self.timeout_seconds)
            || !(1..=86400).contains(&self.stale_after_seconds)
        {
            return Err(Error("invalid_fleet_config"));
        }
        let mut ids = BTreeSet::new();
        for h in &self.hosts {
            if !label(&h.host_id)
                || !ids.insert(&h.host_id)
                || !fingerprint(&h.configuration_fingerprint)
            {
                return Err(Error("invalid_fleet_host"));
            }
            let (exe, cfg, cache) = match &h.transport {
                Transport::Local {
                    executable,
                    config,
                    cache,
                } => {
                    if !std::path::Path::new(executable).is_absolute()
                        || !std::path::Path::new(config).is_absolute()
                        || cache
                            .as_ref()
                            .is_some_and(|c| !std::path::Path::new(c).is_absolute())
                    {
                        return Err(Error("fleet_path_must_be_absolute"));
                    }
                    (executable, config, cache)
                }
                Transport::Ssh {
                    destination,
                    executable,
                    config,
                    cache,
                    ..
                } => {
                    if destination.is_empty()
                        || destination.len() > 255
                        || destination.starts_with('-')
                        || !destination
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"._-@:".contains(&b))
                    {
                        return Err(Error("invalid_ssh_destination"));
                    }
                    (executable, config, cache)
                }
            };
            if [Some(exe), Some(cfg), cache.as_ref()]
                .into_iter()
                .flatten()
                .any(|s| s.is_empty() || s.len() > 4096 || s.chars().any(char::is_control))
            {
                return Err(Error("invalid_fleet_path"));
            }
        }
        Ok(())
    }
    pub fn fingerprint(&self) -> Result<String> {
        self.validate()?;
        Ok(hash(
            &serde_json::to_vec(self).map_err(|_| Error("invalid_fleet_config"))?,
        ))
    }
}
pub fn failure_code(value: &str) -> Option<&'static str> {
    [
        "collection_failed",
        "collection_timeout",
        "collection_interrupted",
        "invalid_snapshot",
        "fleet_snapshot_identity_mismatch",
        "collector_launch_failed",
        "collector_output_too_large",
        "configuration_changed",
        "source_changed_during_scan",
        "collection_busy",
        "source_missing",
        "source_unreadable",
        "invalid_token_field",
        "unsupported_sqlite_schema",
        "invalid_cache",
        "invalid_sqlite_json",
        "conflicting_response_replay",
        "mixed_codex_storage",
    ]
    .into_iter()
    .find(|v| *v == value)
}
pub fn validate_snapshot(s: &Snapshot) -> Result<()> {
    output::summary(s)?;
    if !fingerprint(&s.configuration_fingerprint)
        || !label(&s.parser_version)
        || !label(&s.collector_version)
        || s.cost.is_some()
        || s.sources.is_empty()
    {
        return Err(Error("invalid_snapshot"));
    }
    let mut sources = BTreeSet::new();
    if s.sources.iter().any(|s| !sources.insert(&s.source_id)) {
        return Err(Error("invalid_snapshot"));
    }
    Ok(())
}
#[derive(Debug, Serialize, Deserialize)]
pub struct HostView {
    pub host_id: String,
    pub state: String,
    pub included: bool,
    pub last_success_at: Option<String>,
    pub last_attempt_at: Option<String>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_source_id: Option<String>,
    pub snapshot: Option<Snapshot>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct FleetReport {
    pub schema_version: u32,
    pub kind: String,
    pub configuration_fingerprint: String,
    pub generated_at: String,
    pub complete: bool,
    pub coverage_complete: bool,
    pub totals: Option<Tokens>,
    pub hosts: Vec<HostView>,
}

/// Bounded synchronous orchestration. Run this CLI off the GUI thread.
/// At most two collectors execute concurrently, each with its own deadline.
pub fn run(
    config_path: &std::path::Path,
    store_path: &std::path::Path,
    refresh: bool,
) -> Result<FleetReport> {
    use chrono::Utc;
    use std::{fs::File, time::Duration};
    fn read(path: &std::path::Path) -> Result<FleetConfig> {
        let cfg: FleetConfig = serde_json::from_reader(
            File::open(path).map_err(|_| Error("fleet_config_read_failed"))?,
        )
        .map_err(|_| Error("invalid_fleet_config"))?;
        cfg.validate()?;
        Ok(cfg)
    }
    let config = read(config_path)?;
    let expected = config.fingerprint()?;
    if !refresh {
        return store::Store::read_only(store_path)?.report(&config, Utc::now());
    }
    let config_target =
        std::fs::canonicalize(config_path).map_err(|_| Error("fleet_config_read_failed"))?;
    let store_target = crate::output::destination(store_path)?;
    if store_target == config_target {
        return Err(Error("state_path_conflict"));
    }
    // Central state must be separate from local collector config/cache dirs:
    // otherwise the fleet lock would deadlock its own child collector.
    for host in &config.hosts {
        if let Transport::Local {
            executable,
            config: source_config,
            cache,
        } = &host.transport
        {
            for resource in [Some(source_config), cache.as_ref()].into_iter().flatten() {
                let dest = crate::output::destination(std::path::Path::new(resource))?;
                if dest.parent() == store_target.parent() || dest.parent() == config_target.parent()
                {
                    return Err(Error("fleet_state_overlaps_collector"));
                }
            }
            if crate::output::destination(std::path::Path::new(executable))? == store_target {
                return Err(Error("state_path_conflict"));
            }
            // Missing/broken local source configuration is a host failure, not
            // a reason to prevent independent hosts from refreshing.
            if let Ok(file) = File::open(source_config) {
                if let Ok(source) = serde_json::from_reader::<_, Config>(file) {
                    for source in &source.sources {
                        for path in &source.paths {
                            let root = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
                            if crate::output::source_contains(&root, &store_target) {
                                return Err(Error("output_inside_source"));
                            }
                        }
                    }
                }
            }
        }
    }
    let _locks = crate::locking::CollectionLocks::acquire_state(&[config_path, store_path])?;
    if read(config_path)?.fingerprint()? != expected {
        return Err(Error("configuration_changed"));
    }
    let mut store = store::Store::open(&store_target)?;
    let generation = store.activate(&config)?;
    for batch in config.hosts.chunks(2) {
        let attempts = batch
            .iter()
            .map(|h| store.begin(&h.host_id, generation, Utc::now()))
            .collect::<Result<Vec<_>>>()?;
        let results = std::thread::scope(|scope| {
            let workers: Vec<_> = batch
                .iter()
                .map(|host| {
                    scope.spawn(|| {
                        let result = transport::collect_host_detailed(
                            host,
                            Duration::from_secs(config.timeout_seconds),
                        );
                        (Utc::now(), result)
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|w| {
                    w.join()
                        .unwrap_or((Utc::now(), Err(Error("collection_failed").into())))
                })
                .collect::<Vec<_>>()
        });
        for (attempt, (received_at, result)) in attempts.iter().zip(results) {
            if read(config_path)?.fingerprint()? != expected {
                return Err(Error("configuration_changed"));
            }
            match result {
                Ok(snapshot) => store.accept(attempt, &snapshot, received_at)?,
                Err(error) => {
                    let code = failure_code(error.error.0).unwrap_or("collection_failed");
                    store.fail_with_source(attempt, code, error.source_id.as_deref())?;
                }
            }
        }
    }
    if read(config_path)?.fingerprint()? != expected {
        return Err(Error("configuration_changed"));
    }
    store.report(&config, Utc::now())
}
