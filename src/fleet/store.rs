use super::*;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::{path::Path, time::Duration};
fn db<T>(r: rusqlite::Result<T>) -> Result<T> {
    r.map_err(|_| Error("fleet_store_failed"))
}
#[derive(Debug, Clone)]
pub struct Attempt {
    host: String,
    generation: i64,
    serial: i64,
    fingerprint: String,
}
type StoredHost = (
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);
pub struct Store {
    conn: Connection,
    has_failed_source: bool,
}
impl Store {
    pub fn read_only(path: &Path) -> Result<Self> {
        let conn = crate::sqlite::open_readonly(path).map_err(|_| Error("fleet_store_failed"))?;
        let marker: i64 = db(conn.query_row("PRAGMA application_id", [], |r| r.get(0)))?;
        if marker != 0x41555347 {
            return Err(Error("not_fleet_store"));
        }
        let schema: i64 = db(conn.query_row("PRAGMA user_version", [], |r| r.get(0)))?;
        if schema > 2 {
            return Err(Error("unsupported_fleet_store_schema"));
        }
        let has_failed_source: bool = db(conn.query_row(
            "SELECT count(*)>0 FROM pragma_table_info('fleet_hosts') WHERE name='failed_source'",
            [],
            |r| r.get(0),
        ))?;
        Ok(Self {
            conn,
            has_failed_source,
        })
    }

    pub fn open(path: &Path) -> Result<Self> {
        if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(Error("fleet_store_symlink"));
        }
        let mut conn = db(Connection::open(path))?;
        db(conn.busy_timeout(Duration::from_secs(3)))?;
        // This is app-owned state, never a source database. Require an empty DB
        // or our marker before creating tables; refuse unrelated databases.
        let version: i64 = db(conn.query_row("PRAGMA application_id", [], |r| r.get(0)))?;
        let tables: i64 = db(conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        ))?;
        if (version != 0 && version != 0x41555347) || (version == 0 && tables != 0) {
            return Err(Error("not_fleet_store"));
        }
        let schema: i64 = db(conn.query_row("PRAGMA user_version", [], |r| r.get(0)))?;
        if schema > 2 {
            return Err(Error("unsupported_fleet_store_schema"));
        }
        let tx = db(conn.transaction_with_behavior(TransactionBehavior::Immediate))?;
        db(tx.execute_batch("PRAGMA trusted_schema=OFF;PRAGMA foreign_keys=ON;CREATE TABLE IF NOT EXISTS fleet_meta(id INTEGER PRIMARY KEY CHECK(id=1),generation INTEGER NOT NULL,configuration TEXT NOT NULL);CREATE TABLE IF NOT EXISTS fleet_hosts(host TEXT PRIMARY KEY,generation INTEGER NOT NULL,fingerprint TEXT NOT NULL,attempt INTEGER NOT NULL DEFAULT 0,last_attempt TEXT,error TEXT,snapshot TEXT,success_generation INTEGER,received TEXT);PRAGMA application_id=1096110919;"))?;
        let has_failed_source: bool = db(tx.query_row(
            "SELECT count(*)>0 FROM pragma_table_info('fleet_hosts') WHERE name='failed_source'",
            [],
            |r| r.get(0),
        ))?;
        if !has_failed_source {
            db(tx.execute_batch("ALTER TABLE fleet_hosts ADD COLUMN failed_source TEXT"))?;
        }
        db(tx.execute_batch("PRAGMA user_version=2"))?;
        db(tx.commit())?;
        Ok(Self {
            conn,
            has_failed_source: true,
        })
    }
    /// Activation is one transaction. Every semantic config edit gets a fresh
    /// generation, including A -> B -> A, fencing all outstanding old attempts.
    pub fn activate(&mut self, config: &FleetConfig) -> Result<i64> {
        let fingerprint = config.fingerprint()?;
        let tx = db(self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate))?;
        let previous: Option<(i64, String)> = db(tx
            .query_row(
                "SELECT generation,configuration FROM fleet_meta WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional())?;
        if let Some((g, f)) = &previous {
            if *f == fingerprint {
                return Ok(*g);
            }
        }
        let generation = previous.map_or(Ok(1), |(g, _)| {
            g.checked_add(1).ok_or(Error("fleet_generation_overflow"))
        })?;
        db(tx.execute("INSERT INTO fleet_meta VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET generation=excluded.generation,configuration=excluded.configuration",params![generation,fingerprint]))?;
        for host in &config.hosts {
            db(tx.execute("INSERT INTO fleet_hosts(host,generation,fingerprint) VALUES(?1,?2,?3) ON CONFLICT(host) DO UPDATE SET generation=excluded.generation,fingerprint=excluded.fingerprint,error=NULL,failed_source=NULL,last_attempt=NULL",params![host.host_id,generation,host.configuration_fingerprint]))?;
        }
        db(tx.execute("DELETE FROM fleet_hosts WHERE generation<>?1", [generation]))?;
        db(tx.commit())?;
        Ok(generation)
    }
    pub fn begin(&mut self, host: &str, generation: i64, now: DateTime<Utc>) -> Result<Attempt> {
        let tx = db(self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate))?;
        let current: Option<(i64, String)> = db(tx
            .query_row(
                "SELECT attempt,fingerprint FROM fleet_hosts WHERE host=?1 AND generation=?2",
                params![host, generation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional())?;
        let (serial, fingerprint) = current.ok_or(Error("fleet_configuration_changed"))?;
        let serial = serial
            .checked_add(1)
            .ok_or(Error("fleet_attempt_overflow"))?;
        db(tx.execute("UPDATE fleet_hosts SET attempt=?1,last_attempt=?2,error='collection_interrupted',failed_source=NULL WHERE host=?3 AND generation=?4",params![serial,now.to_rfc3339(),host,generation]))?;
        db(tx.commit())?;
        Ok(Attempt {
            host: host.into(),
            generation,
            serial,
            fingerprint,
        })
    }
    pub fn accept(
        &mut self,
        attempt: &Attempt,
        snapshot: &Snapshot,
        now: DateTime<Utc>,
    ) -> Result<()> {
        validate_snapshot(snapshot)?;
        if snapshot.host_id != attempt.host
            || snapshot.configuration_fingerprint != attempt.fingerprint
        {
            return Err(Error("fleet_snapshot_identity_mismatch"));
        }
        let json = serde_json::to_string(snapshot).map_err(|_| Error("invalid_snapshot"))?;
        let n=db(self.conn.execute("UPDATE fleet_hosts SET snapshot=?1,success_generation=?2,received=?3,error=NULL,failed_source=NULL WHERE host=?4 AND generation=?2 AND attempt=?5",params![json,attempt.generation,now.to_rfc3339(),attempt.host,attempt.serial]))?;
        if n != 1 {
            return Err(Error("fleet_stale_result"));
        }
        Ok(())
    }
    pub fn fail(&mut self, attempt: &Attempt, code: &str) -> Result<()> {
        self.fail_with_source(attempt, code, None)
    }
    pub fn fail_with_source(
        &mut self,
        attempt: &Attempt,
        code: &str,
        source: Option<&str>,
    ) -> Result<()> {
        if source.is_some_and(|id| !label(id)) {
            return Err(Error("invalid_fleet_source"));
        }
        if failure_code(code).is_none() {
            return Err(Error("invalid_fleet_error"));
        }
        let n = db(self.conn.execute(
            "UPDATE fleet_hosts SET error=?1,failed_source=?5 WHERE host=?2 AND generation=?3 AND attempt=?4",
            params![code, attempt.host, attempt.generation, attempt.serial,source],
        ))?;
        if n != 1 {
            return Err(Error("fleet_stale_result"));
        }
        Ok(())
    }
    pub fn report(&mut self, config: &FleetConfig, now: DateTime<Utc>) -> Result<FleetReport> {
        let expected = config.fingerprint()?;
        let tx = db(self.conn.transaction())?;
        let (generation, fingerprint): (i64, String) = db(tx.query_row(
            "SELECT generation,configuration FROM fleet_meta WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ))?;
        if fingerprint != expected {
            return Err(Error("fleet_configuration_changed"));
        }
        let mut hosts = Vec::new();
        let mut total = Tokens::default();
        let mut any = false;
        let mut complete = true;
        let mut coverage = true;
        for h in &config.hosts {
            let sql = if self.has_failed_source {
                "SELECT snapshot,success_generation,received,last_attempt,error,failed_source FROM fleet_hosts WHERE host=?1 AND generation=?2"
            } else {
                "SELECT snapshot,success_generation,received,last_attempt,error,NULL FROM fleet_hosts WHERE host=?1 AND generation=?2"
            };
            let (json,success_generation,received,last_attempt,error,failed_source_id):StoredHost=db(tx.query_row(sql,params![h.host_id,generation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))))?;
            if failed_source_id.as_ref().is_some_and(|id| !label(id)) {
                return Err(Error("invalid_stored_source"));
            }
            let snapshot = json
                .map(|j| {
                    serde_json::from_str::<Snapshot>(&j)
                        .map_err(|_| Error("invalid_stored_snapshot"))
                })
                .transpose()?;
            if let Some(s) = &snapshot {
                validate_snapshot(s)?;
            }
            let included = success_generation == Some(generation)
                && snapshot.as_ref().is_some_and(|s| {
                    s.host_id == h.host_id
                        && s.configuration_fingerprint == h.configuration_fingerprint
                });
            let age = received
                .as_ref()
                .map(|t| {
                    DateTime::parse_from_rfc3339(t)
                        .map(|t| now.signed_duration_since(t).num_seconds())
                        .map_err(|_| Error("invalid_stored_time"))
                })
                .transpose()?;
            let state = if snapshot.is_none() {
                "never_collected"
            } else if !included {
                "configuration_changed"
            } else if error.is_some()
                || age.is_none_or(|a| a < 0 || a > config.stale_after_seconds as i64)
            {
                "stale"
            } else {
                "fresh"
            };
            complete &= state == "fresh";
            if included {
                let s = snapshot.as_ref().unwrap();
                total = total.checked_add(s.totals)?;
                any = true;
                coverage &= s.coverage_complete;
            } else {
                coverage = false;
            }
            hosts.push(HostView {
                host_id: h.host_id.clone(),
                state: state.into(),
                included,
                last_success_at: received,
                last_attempt_at: last_attempt,
                error,
                failed_source_id,
                snapshot,
            });
        }
        db(tx.rollback())?;
        Ok(FleetReport {
            schema_version: 1,
            kind: "fleet_snapshot".into(),
            configuration_fingerprint: fingerprint,
            generated_at: now.to_rfc3339(),
            complete,
            coverage_complete: coverage && complete,
            totals: any.then_some(total),
            hosts,
        })
    }
}
