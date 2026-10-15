//! Cooperative CLI serialization. Persistent empty markers keep lock identity
//! stable across atomic cache/snapshot replacement and process termination.
use crate::{output, Config, Error, Result};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
};

pub const LOCK_FILE_NAME: &str = ".agent-usage.lock";

/// Held from before collection until publication completes. Locks are directory
/// scoped: configurations/state files sharing a directory intentionally serialize.
/// Callers must keep the markers in place; removing one could split lock identity.
#[derive(Debug)]
pub struct CollectionLocks {
    _files: Vec<File>,
}

impl CollectionLocks {
    /// Serialize orchestration processes sharing a config or state directory.
    pub fn acquire_state(paths: &[&Path]) -> Result<Self> {
        let resources: Vec<PathBuf> = paths
            .iter()
            .map(|p| output::destination(p))
            .collect::<Result<_>>()?;
        let mut markers = BTreeSet::new();
        for path in &resources {
            let marker = path
                .parent()
                .ok_or(Error("invalid_lock_path"))?
                .join(LOCK_FILE_NAME);
            if resources.contains(&marker) {
                return Err(Error("lock_path_conflict"));
            }
            markers.insert(marker);
        }
        let mut files = Vec::new();
        for marker in markers {
            files.push(acquire_one(&marker)?);
        }
        Ok(Self { _files: files })
    }

    pub fn acquire(
        config_path: &Path,
        cache: Option<&Path>,
        target: Option<&Path>,
        config: &Config,
    ) -> Result<Self> {
        let config_path = fs::canonicalize(config_path).map_err(|_| Error("config_read_failed"))?;
        let mut resources = vec![config_path];
        for path in [cache, target].into_iter().flatten() {
            let path = output::destination(path)?;
            if resources.contains(&path) {
                return Err(Error("state_path_conflict"));
            }
            resources.push(path);
        }
        let roots: Vec<PathBuf> = config
            .sources
            .iter()
            .flat_map(|s| &s.paths)
            .map(|p| {
                fs::canonicalize(p).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        Error("source_missing")
                    } else {
                        Error("source_unreadable")
                    }
                })
            })
            .collect::<Result<_>>()?;
        let mut markers = BTreeSet::new();
        for resource in &resources {
            let dir = resource.parent().ok_or(Error("invalid_lock_path"))?;
            if roots.iter().any(|root| dir.starts_with(root)) {
                return Err(Error("lock_inside_source"));
            }
            let marker = dir.join(LOCK_FILE_NAME);
            if resources.contains(&marker) {
                return Err(Error("lock_path_conflict"));
            }
            markers.insert(marker);
        }
        // Validate the full plan before creating any markers. Every operation is
        // nonblocking; partial acquisitions are released automatically on error.
        let mut files = Vec::new();
        for marker in markers {
            files.push(acquire_one(&marker)?);
        }
        Ok(Self { _files: files })
    }
}

fn acquire_one(path: &Path) -> Result<File> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.is_file() || metadata.len() != 0 {
            return Err(Error("invalid_lock_file"));
        }
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|_| Error("lock_open_failed"))?;
    let opened = file.metadata().map_err(|_| Error("lock_metadata_failed"))?;
    let current = fs::symlink_metadata(path).map_err(|_| Error("lock_metadata_failed"))?;
    if !opened.is_file() || opened.len() != 0 || !current.is_file() || current.len() != 0 {
        return Err(Error("invalid_lock_file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != current.dev() || opened.ino() != current.ino() || opened.nlink() != 1 {
            return Err(Error("invalid_lock_file"));
        }
    }
    file.try_lock().map_err(|e| match e {
        TryLockError::WouldBlock => Error("collection_busy"),
        TryLockError::Error(_) => Error("lock_failed"),
    })?;
    Ok(file)
}
