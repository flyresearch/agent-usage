use agent_usage::{collect, output, Config, Error, Snapshot};
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{self, Write},
    path::PathBuf,
};

fn run() -> Result<Value, Error> {
    let mut args = std::env::args_os().skip(1);
    let command = args.next().ok_or(Error("missing_command"))?;
    if command == "--help" || command == "help" {
        if args.next().is_some() {
            return Err(Error("invalid_argument"));
        }
        return Ok(json!({"schema_version":1,"kind":"help","commands":[
            "collect --config PRIVATE_CONFIG [--cache PRIVATE_CACHE] [--output PRIVATE_SNAPSHOT] [--summary]",
            "report --snapshot PRIVATE_SNAPSHOT"],"notes":["Source directories must be explicit absolute paths.","Output and cache parents must exist.","Failed collection does not replace the saved snapshot.","Known totals may exclude unmetered or unclassified history; inspect coverage."]}));
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
    let config: Config =
        serde_json::from_reader(File::open(&config_path).map_err(|_| Error("config_read_failed"))?)
            .map_err(|_| Error("invalid_config"))?;
    let target = target
        .map(|p| output::validate_destination(&p, &config_path, cache.as_deref(), &config))
        .transpose()?;
    let snapshot = collect(&config, cache.as_deref())?;
    let value = if summary {
        output::summary(&snapshot)?
    } else {
        serde_json::to_value(&snapshot).map_err(|_| Error("serialization_failed"))?
    };
    if let Some(path) = target {
        output::publish(&path, &snapshot)?;
    }
    Ok(value)
}
fn main() {
    let (value, code) = match run() {
        Ok(value) => (value, 0),
        Err(e) => {
            eprintln!("agent-usage: {}", e.0);
            (json!({"schema_version":1,"complete":false,"error":e.0}), 1)
        }
    };
    let mut out = io::stdout().lock();
    if serde_json::to_writer(&mut out, &value).is_err() || writeln!(out).is_err() {
        std::process::exit(2);
    }
    std::process::exit(code);
}
