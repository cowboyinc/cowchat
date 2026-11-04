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
    while mapping_path.exists() {
        let (mapping, session) = load(mapping_path)?;
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
                "60",
                "--cursor-file",
            ])
            .arg(&temporary)
            .args(["--idle-timeout", "300"])
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
            if temporary.exists() {
                std::fs::rename(&temporary, &cursor)?;
            }
        } else {
            let _ = std::fs::remove_file(&temporary);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
    Ok(())
}
