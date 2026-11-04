use cowchat_codex::app_server::{CodexAppServerClient, WakeBackend, WakeReference};
use cowchat_codex::config::CodexConfig;
use cowchat_core::ChatMessage;
use fs2::FileExt;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

#[derive(Deserialize)]
struct Mapping {
    thread_id: String,
    id: String,
    name: String,
    session_file: PathBuf,
    parent_pid: u32,
    parent_started: String,
}

fn parent_alive(mapping: &Mapping) -> bool {
    std::process::Command::new("ps")
        .args(["-o", "lstart=", "-p", &mapping.parent_pid.to_string()])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| {
            String::from_utf8_lossy(&output.stdout).trim() == mapping.parent_started
        })
}

#[derive(Deserialize)]
struct SessionFile {
    id: String,
    url: String,
    room_id: String,
    session_token: String,
}

fn load(mapping_path: &Path) -> Result<(Mapping, SessionFile), Box<dyn std::error::Error>> {
    let meta = std::fs::metadata(mapping_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err("Codex session mapping permissions are too broad".into());
        }
    }
    let mapping: Mapping = serde_json::from_slice(&std::fs::read(mapping_path)?)?;
    let session_meta = std::fs::metadata(&mapping.session_file)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if session_meta.permissions().mode() & 0o077 != 0 {
            return Err("Codex session token permissions are too broad".into());
        }
    }
    if !session_meta.is_file() {
        return Err("Codex session token is not a file".into());
    }
    let session: SessionFile = serde_json::from_slice(&std::fs::read(&mapping.session_file)?)?;
    if mapping.id != session.id || !session.session_token.starts_with("csoc_") {
        return Err("Codex session mapping does not match its token".into());
    }
    uuid::Uuid::parse_str(&mapping.thread_id)?;
    Ok((mapping, session))
}

pub async fn listen(mapping_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let lock_path = mapping_path.with_extension("lock");
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    if let Err(error) = lock.try_lock_exclusive() {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            return Ok(());
        }
        return Err(error.into());
    }
    let home = directories::BaseDirs::new()
        .ok_or("cannot locate home")?
        .home_dir()
        .to_path_buf();
    let mut config = CodexConfig::default();
    config.app_server_endpoint = config
        .app_server_endpoint
        .replace("~/", &format!("{}/", home.display()));
    let codex = CodexAppServerClient::new(config);
    let cursor = mapping_path.with_extension("cursor.json");
    let mut wake_failures = 0u8;
    while mapping_path.exists() {
        let (mapping, session) = load(mapping_path)?;
        if !parent_alive(&mapping) {
            return Err("Codex parent process exited".into());
        }
        match codex.thread_exists(&mapping.thread_id).await {
            Ok(false) => return Err("Codex thread no longer exists".into()),
            Err(error) => {
                eprintln!("Cowchat Codex thread probe failed: {error}");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
            Ok(true) => {}
        }
        let mut heartbeat_command = Command::new("cowchat");
        heartbeat_command.kill_on_drop(true);
        let heartbeat = tokio::time::timeout(
            Duration::from_secs(15),
            heartbeat_command
                .args([
                    "--url",
                    &session.url,
                    "--name",
                    &mapping.name,
                    "--agent-id",
                    &mapping.id,
                    "session",
                    "heartbeat",
                    &mapping.id,
                ])
                .env("COWCHAT_SESSION_FILE", &mapping.session_file)
                .output(),
        )
        .await??;
        if !heartbeat.status.success() {
            return Err("Cowchat session heartbeat failed or session ended".into());
        }
        let temporary = mapping_path.with_extension(format!("{}.cursor.tmp", uuid::Uuid::new_v4()));
        if cursor.exists() {
            std::fs::copy(&cursor, &temporary)?;
        }
        let mut wait = Command::new("cowchat");
        wait.kill_on_drop(true);
        let output = wait
            .args([
                "--url",
                &session.url,
                "--name",
                &mapping.name,
                "--agent-id",
                &mapping.id,
                "wait",
                &session.room_id,
                "--approved-rooms",
                "--mentions-only",
                "--loop",
                "--heartbeat-secs",
                "0",
                "--cursor-file",
            ])
            .arg(&temporary)
            .args(["--idle-timeout", "60"])
            .env("COWCHAT_SESSION_FILE", &mapping.session_file)
            .output();
        let output = match tokio::time::timeout(Duration::from_secs(360), output).await {
            Ok(result) => result?,
            Err(_) => {
                let _ = std::fs::remove_file(&temporary);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let messages = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() && !matches!(output.status.code(), Some(2 | 3)) {
            let error = String::from_utf8_lossy(&output.stderr);
            if error.contains("401 Unauthorized") || error.contains("404 Not Found") {
                let _ = std::fs::remove_file(&temporary);
                return Err("Cowchat session ended or was revoked".into());
            }
        }
        let mut delivered = true;
        for line in messages.lines() {
            let message: ChatMessage = match serde_json::from_str(line) {
                Ok(message) => message,
                Err(_) => continue,
            };
            let reference = WakeReference {
                target: "codex-session".into(),
                room: message.room_id.clone(),
                after_seq: message.seq.saturating_sub(1),
                observed_seq: message.seq,
                source: "cowchat".into(),
                event_id: message.message_id.clone(),
                event_type: "cowchat.message".into(),
            };
            if let Err(error) = codex.wake(&mapping.thread_id, &reference).await {
                eprintln!("Cowchat Codex wake failed: {error}");
                delivered = false;
                break;
            }
        }
        if delivered && (output.status.success() || matches!(output.status.code(), Some(2 | 3))) {
            wake_failures = 0;
            if temporary.exists() {
                std::fs::rename(&temporary, &cursor)?;
            }
        } else {
            let _ = std::fs::remove_file(&temporary);
            wake_failures = wake_failures.saturating_add(1);
            if wake_failures >= 12 {
                return Err(
                    "Cowchat Codex listener stopped after 12 consecutive wake failures".into(),
                );
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_parent_cannot_keep_session_alive() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id();
        let started = std::process::Command::new("ps")
            .args(["-o", "lstart=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        let mapping = Mapping {
            thread_id: String::new(),
            id: String::new(),
            name: String::new(),
            session_file: PathBuf::new(),
            parent_pid: pid,
            parent_started: String::from_utf8_lossy(&started.stdout).trim().to_string(),
        };
        assert!(parent_alive(&mapping));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!parent_alive(&mapping));
    }
}
