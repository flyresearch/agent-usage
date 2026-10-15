//! Source databases are opened readonly with WAL-aware SQLite connections.
//! No immutable mode, journal changes, checkpoint, credentials, or extensions.
pub mod agy;
pub mod hermes;
pub mod openclaw;
mod wire;
use crate::{Error, Result};
use rusqlite::{Connection, OpenFlags};
use std::{path::Path, time::Duration};

pub fn open_readonly(path: &Path) -> Result<Connection> {
    // Explicitly configured aliases are resolved before NOFOLLOW protects the
    // final SQLite open (temporary-directory aliases are common on macOS).
    let resolved = std::fs::canonicalize(path).map_err(|_| Error("sqlite_open_failed"))?;
    let conn = Connection::open_with_flags(
        resolved,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|_| Error("sqlite_open_failed"))?;
    conn.busy_timeout(Duration::from_secs(3))
        .map_err(|_| Error("sqlite_setup_failed"))?;
    conn.execute_batch("PRAGMA query_only = ON; PRAGMA trusted_schema = OFF;")
        .map_err(|_| Error("sqlite_setup_failed"))?;
    Ok(conn)
}
