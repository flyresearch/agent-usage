use super::*;
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
const MAX_OUTPUT: usize = 64 * 1024 * 1024;
fn posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
fn powershell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for part in bytes.chunks(3) {
        let n = ((part[0] as u32) << 16)
            | ((part.get(1).copied().unwrap_or(0) as u32) << 8)
            | part.get(2).copied().unwrap_or(0) as u32;
        for shift in [18, 12, 6, 0] {
            out.push(ALPHABET[((n >> shift) & 63) as usize] as char);
        }
        if part.len() < 3 {
            out.pop();
            out.push('=');
        }
        if part.len() < 2 {
            out.pop();
            out.pop();
            out.push_str("==");
        }
    }
    out
}
/// SSH always invokes a remote shell. Quote POSIX words or encode a complete
/// PowerShell script so no path text is interpreted by an outer Windows shell.
pub fn remote_command(
    executable: &str,
    config: &str,
    cache: Option<&str>,
    shell: RemoteShell,
) -> String {
    let mut args = vec![executable, "collect", "--config", config];
    if let Some(cache) = cache {
        args.extend(["--cache", cache]);
    }
    match shell {
        RemoteShell::Posix => args.into_iter().map(posix).collect::<Vec<_>>().join(" "),
        RemoteShell::Powershell => {
            let script = format!(
                "& {}; exit $LASTEXITCODE",
                args.into_iter()
                    .map(powershell)
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
            format!(
                "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {}",
                base64(&bytes)
            )
        }
    }
}
fn command(host: &HostConfig) -> Command {
    let mut command = match &host.transport {
        Transport::Local {
            executable,
            config,
            cache,
        } => {
            let mut c = Command::new(executable);
            c.args(["collect", "--config", config]);
            if let Some(cache) = cache {
                c.args(["--cache", cache]);
            }
            c
        }
        Transport::Ssh {
            destination,
            executable,
            config,
            cache,
            shell,
        } => {
            #[cfg(windows)]
            let ssh = std::path::PathBuf::from(
                std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
            )
            .join("System32/OpenSSH/ssh.exe");
            #[cfg(not(windows))]
            let ssh = std::path::PathBuf::from("/usr/bin/ssh");
            let mut c = Command::new(ssh);
            c.args([
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "StrictHostKeyChecking=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "ServerAliveInterval=10",
                "-o",
                "ServerAliveCountMax=2",
                "--",
                destination,
            ]);
            c.arg(remote_command(executable, config, cache.as_deref(), *shell));
            c
        }
    };
    command.env_clear();
    for key in [
        "HOME",
        "USERPROFILE",
        "SystemRoot",
        "WINDIR",
        "PATH",
        "TMPDIR",
        "TEMP",
        "TMP",
        "SSH_AUTH_SOCK",
    ] {
        if let Some(v) = std::env::var_os(key) {
            command.env(key, v);
        }
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}
pub fn collect_host(host: &HostConfig, timeout: Duration) -> Result<Snapshot> {
    collect_host_detailed(host, timeout).map_err(|failure| failure.error)
}
pub fn collect_host_detailed(
    host: &HostConfig,
    timeout: Duration,
) -> std::result::Result<Snapshot, CollectionFailure> {
    let mut failed_source = None;
    collect_host_inner(host, timeout, &mut failed_source).map_err(|error| CollectionFailure {
        error,
        source_id: failed_source,
    })
}
fn collect_host_inner(
    host: &HostConfig,
    timeout: Duration,
    failed_source: &mut Option<String>,
) -> Result<Snapshot> {
    FleetConfig {
        schema_version: 1,
        hosts: vec![host.clone()],
        timeout_seconds: 1,
        stale_after_seconds: 1,
    }
    .validate()?;
    let mut child = command(host)
        .spawn()
        .map_err(|_| Error("collector_launch_failed"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or(Error("collector_launch_failed"))?;
    let overflow = Arc::new(AtomicBool::new(false));
    let signal = overflow.clone();
    // The reader is bounded. A child retaining stdout after its parent exits
    // cannot block timeout handling: only join after a channel result arrives.
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut data = Vec::new();
        let mut buffer = [0u8; 8192];
        let result = (|| loop {
            let n = stdout
                .read(&mut buffer)
                .map_err(|_| Error("collection_failed"))?;
            if n == 0 {
                return Ok(data);
            }
            if data.len() + n > MAX_OUTPUT {
                signal.store(true, Ordering::Release);
                return Err(Error("collector_output_too_large"));
            }
            data.extend_from_slice(&buffer[..n]);
        })();
        let _ = send.send(result);
    });
    let start = Instant::now();
    let mut status = None;
    let mut payload = None;
    loop {
        if overflow.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error("collector_output_too_large"));
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error("collection_timeout"));
        }
        if payload.is_none() {
            match receive.try_recv() {
                Ok(result) => payload = Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error("collection_failed"));
                }
                Err(_) => {}
            }
        }
        if status.is_none() {
            status = child.try_wait().map_err(|_| Error("collection_failed"))?;
        }
        if let (Some(exit), Some(result)) = (&status, &payload) {
            if !exit.success() {
                if let Some(envelope) = result
                    .as_ref()
                    .ok()
                    .and_then(|data| serde_json::from_slice::<serde_json::Value>(data).ok())
                {
                    if envelope["schema_version"] == 1 && envelope["complete"] == false {
                        *failed_source = envelope["source_id"]
                            .as_str()
                            .filter(|s| label(s))
                            .map(str::to_owned);
                    }
                }
                let code = result
                    .as_ref()
                    .ok()
                    .and_then(|data| serde_json::from_slice::<serde_json::Value>(data).ok())
                    .and_then(|v| {
                        v.get("error")
                            .and_then(|e| e.as_str())
                            .and_then(failure_code)
                    });
                return Err(Error(code.unwrap_or("collection_failed")));
            }
            let data = result.as_ref().map_err(|e| *e)?;
            let snapshot: Snapshot =
                serde_json::from_slice(data).map_err(|_| Error("invalid_snapshot"))?;
            validate_snapshot(&snapshot)?;
            if snapshot.host_id != host.host_id
                || snapshot.configuration_fingerprint != host.configuration_fingerprint
            {
                return Err(Error("fleet_snapshot_identity_mismatch"));
            }
            return Ok(snapshot);
        }
        thread::sleep(Duration::from_millis(10));
    }
}
