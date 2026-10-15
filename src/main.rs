use agent_usage::{collector, locking::CollectionLocks, output, Config, Error, Snapshot};
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
};

fn run(failed_source: &mut Option<String>) -> Result<Value, Error> {
    let mut args = std::env::args_os().skip(1);
    let command = args.next().ok_or(Error("missing_command"))?;
    if command == "--help" || command == "help" {
        if args.next().is_some() {
            return Err(Error("invalid_argument"));
        }
        return Ok(json!({"schema_version":1,"kind":"help","commands":[
            "collect --config PRIVATE_CONFIG [--cache PRIVATE_CACHE] [--output PRIVATE_SNAPSHOT] [--summary]",
            "report --snapshot PRIVATE_SNAPSHOT", "config-id --config PRIVATE_CONFIG", "fleet-sync --config PRIVATE_FLEET_CONFIG --store PRIVATE_FLEET_DB", "fleet-report --config PRIVATE_FLEET_CONFIG --store PRIVATE_FLEET_DB"],"notes":["Source directories or supported database files must be explicit absolute paths.","Config, output and cache directories must exist and allow lock marker creation.","Overlapping collection/state directories return collection_busy; report remains read-only.","Failed collection does not replace the saved snapshot.","Known totals may exclude unmetered or unclassified history; inspect coverage."]}));
    }
    if command == "config-id" {
        if args.next().as_deref() != Some(std::ffi::OsStr::new("--config")) {
            return Err(Error("invalid_argument"));
        }
        let path = PathBuf::from(args.next().ok_or(Error("missing_config_argument"))?);
        if args.next().is_some() {
            return Err(Error("invalid_argument"));
        }
        let config = read_config(&path)?;
        if config.schema_version != 1 || !agent_usage::label(&config.host_id) {
            return Err(Error("invalid_config"));
        }
        return Ok(
            json!({"schema_version":1,"kind":"configuration_identity","host_id":config.host_id,"configuration_fingerprint":agent_usage::hash(&serde_json::to_vec(&config).map_err(|_|Error("invalid_config"))?)}),
        );
    }
    if command == "fleet-sync" || command == "fleet-report" {
        let mut config = None;
        let mut store = None;
        while let Some(flag) = args.next() {
            let slot = match flag.to_str() {
                Some("--config") => &mut config,
                Some("--store") => &mut store,
                _ => return Err(Error("invalid_argument")),
            };
            if slot.is_some() {
                return Err(Error("duplicate_argument"));
            }
            *slot = Some(PathBuf::from(
                args.next().ok_or(Error("missing_argument_value"))?,
            ));
        }
        let config = config.ok_or(Error("missing_config_argument"))?;
        let store = store.ok_or(Error("missing_store_argument"))?;
        return serde_json::to_value(agent_usage::fleet::run(
            &config,
            &store,
            command == "fleet-sync",
        )?)
        .map_err(|_| Error("serialization_failed"));
    }
    if command == "report" {
        if args.next().as_deref() != Some(std::ffi::OsStr::new("--snapshot")) {
            return Err(Error("invalid_argument"));
        }
        let path = args.next().ok_or(Error("missing_snapshot_argument"))?;
        if args.next().is_some() {
            return Err(Error("invalid_argument"));
        }
        let file = File::open(path).map_err(|_| Error("snapshot_read_failed"))?;
        let snapshot: Snapshot =
            serde_json::from_reader(file).map_err(|_| Error("invalid_snapshot"))?;
        return output::summary(&snapshot);
    }
    if command != "collect" {
        return Err(Error("invalid_command"));
    }
    let mut config_path = None;
    let mut cache = None;
    let mut target = None;
    let mut summary = false;
    while let Some(flag) = args.next() {
        let slot = match flag.to_str() {
            Some("--config") => &mut config_path,
            Some("--cache") => &mut cache,
            Some("--output") => &mut target,
            Some("--summary") if !summary => {
                summary = true;
                continue;
            }
            _ => return Err(Error("invalid_argument")),
        };
        if slot.is_some() {
            return Err(Error("duplicate_argument"));
        }
        *slot = Some(PathBuf::from(
            args.next().ok_or(Error("missing_argument_value"))?,
        ));
    }
    let config_path = config_path.ok_or(Error("missing_config_argument"))?;
    let config = read_config(&config_path)?;
    collector::validate_sources(&config).map_err(|failure| {
        *failed_source = failure.source_id;
        failure.error
    })?;
    let target = target
        .map(|p| output::validate_destination(&p, &config_path, cache.as_deref(), &config))
        .transpose()?;
    let _locks =
        CollectionLocks::acquire(&config_path, cache.as_deref(), target.as_deref(), &config)?;
    ensure_config_unchanged(&config_path, &config)?;
    let snapshot = collector::collect_detailed(&config, cache.as_deref()).map_err(|failure| {
        *failed_source = failure.source_id;
        failure.error
    })?;
    let value = if summary {
        output::summary(&snapshot)?
    } else {
        serde_json::to_value(&snapshot).map_err(|_| Error("serialization_failed"))?
    };
    ensure_config_unchanged(&config_path, &config)?;
    if let Some(path) = target {
        output::publish(&path, &snapshot)?;
    }
    Ok(value)
}
fn read_config(path: &Path) -> Result<Config, Error> {
    serde_json::from_reader(File::open(path).map_err(|_| Error("config_read_failed"))?)
        .map_err(|_| Error("invalid_config"))
}

fn ensure_config_unchanged(path: &Path, expected: &Config) -> Result<(), Error> {
    let current = read_config(path).map_err(|_| Error("configuration_changed"))?;
    if serde_json::to_vec(&current).map_err(|_| Error("invalid_config"))?
        != serde_json::to_vec(expected).map_err(|_| Error("invalid_config"))?
    {
        return Err(Error("configuration_changed"));
    }
    Ok(())
}

fn main() {
    let mut failed_source = None;
    let (value, code) = match run(&mut failed_source) {
        Ok(value) => (value, 0),
        Err(e) => {
            eprintln!("agent-usage: {}", e.0);
            (
                json!({"schema_version":1,"complete":false,"error":e.0,"source_id":failed_source}),
                1,
            )
        }
    };
    let mut out = io::stdout().lock();
    if serde_json::to_writer(&mut out, &value).is_err() || writeln!(out).is_err() {
        std::process::exit(2);
    }
    std::process::exit(code);
}
