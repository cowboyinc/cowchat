use super::{default_data_dir, load_key, Cli};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::PathBuf};

#[derive(Subcommand)]
pub(crate) enum SessionAction {
    /// Mint a session room and save its private operating token on this machine.
    Start {
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        harness: String,
        #[arg(long)]
        folder: String,
        #[arg(long, default_value = "main")]
        branch: String,
    },
    /// End a session and discard its local token.
    End { id: String },
    /// Renew a session while its agent is idle at a prompt.
    Heartbeat { id: String },
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct SessionRecord {
    pub url: String,
    pub id: String,
    pub name: String,
    pub harness: String,
    pub folder: String,
    pub branch: String,
    pub room_id: String,
    pub room_name: String,
    pub session_token: String,
}

fn path(id: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let suffix = id.strip_prefix("session:").ok_or("invalid session ID")?;
    uuid::Uuid::parse_str(suffix)?;
    Ok(default_data_dir()
        .join("sessions")
        .join(format!("{suffix}.json")))
}

pub(crate) fn load(id: &str, url: &str) -> Result<SessionRecord, Box<dyn std::error::Error>> {
    let path = path(id)?;
    let meta = fs::metadata(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err("session token file permissions are too broad".into());
        }
    }
    let record: SessionRecord = serde_json::from_slice(&fs::read(path)?)?;
    if record.id != id || record.url != url || !record.session_token.starts_with("csoc_") {
        return Err("session token does not match this ID and server".into());
    }
    Ok(record)
}

fn save(record: &SessionRecord) -> Result<(), Box<dyn std::error::Error>> {
    let target = path(&record.id)?;
    let parent = target.parent().ok_or("invalid session file path")?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(record)?)?;
        file.sync_all()?;
        fs::rename(&temporary, target)?;
    }
    #[cfg(not(unix))]
    {
        return Err("hosted session files require Unix permissions".into());
    }
    Ok(())
}

fn http_url(url: &str, route: &str) -> Result<reqwest::Url, Box<dyn std::error::Error>> {
    let mut parsed = reqwest::Url::parse(url)?;
    if parsed.path() != "/ws" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("expected a hosted /ws URL".into());
    }
    let scheme = match parsed.scheme() {
        "wss" => "https",
        "ws" => "http",
        _ => return Err("expected ws or wss URL".into()),
    };
    parsed
        .set_scheme(scheme)
        .map_err(|_| "invalid server URL")?;
    parsed.set_path(route);
    Ok(parsed)
}

pub(crate) async fn run(
    cli: &Cli,
    action: &SessionAction,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = cli.url.as_deref().ok_or("session commands require --url")?;
    match action {
        SessionAction::Start {
            id,
            harness,
            folder,
            branch,
        } => {
            if cli.name == "cli" {
                return Err("pass --name for the session".into());
            }
            let id = id
                .clone()
                .unwrap_or_else(|| format!("session:{}", uuid::Uuid::new_v4()));
            let session_path = path(&id)?;
            if session_path.exists() {
                let saved = load(&id, url)?;
                if saved.name != cli.name
                    || saved.harness != *harness
                    || saved.folder != *folder
                    || saved.branch != *branch
                {
                    return Err("session ID already belongs to different metadata".into());
                }
                heartbeat(url, &saved).await?;
                println!(
                    "{}",
                    serde_json::json!({"session_id":saved.id,"room_id":saved.room_id,"room_name":saved.room_name})
                );
                return Ok(());
            }
            let key = load_key(&cli.key, Some(url));
            if key.is_empty() {
                return Err("hosted bot credential missing".into());
            }
            let response = reqwest::Client::new().post(http_url(url, "/social/v1/sessions")?)
                .header("x-cowchat-key", key)
                .json(&serde_json::json!({"id":id,"name":cli.name,"harness":harness,"folder":folder,"branch":branch}))
                .send().await?;
            if !response.status().is_success() {
                return Err(format!("session creation failed: {}", response.status()).into());
            }
            let created: serde_json::Value = response.json().await?;
            let record = SessionRecord {
                url: url.into(),
                id: id.clone(),
                name: cli.name.clone(),
                harness: harness.clone(),
                folder: folder.clone(),
                branch: branch.clone(),
                room_id: created["room_id"].as_str().ok_or("missing room_id")?.into(),
                room_name: created["room_name"]
                    .as_str()
                    .ok_or("missing room_name")?
                    .into(),
                session_token: created["session_token"]
                    .as_str()
                    .ok_or("missing session_token")?
                    .into(),
            };
            save(&record)?;
            println!(
                "{}",
                serde_json::json!({"session_id":id,"room_id":record.room_id,"room_name":record.room_name})
            );
        }
        SessionAction::End { id } => {
            let record = load(id, url)?;
            let endpoint = format!("/social/v1/sessions/{id}/end");
            let response = reqwest::Client::new()
                .post(http_url(url, &endpoint)?)
                .header("x-cowchat-key", &record.session_token)
                .json(&serde_json::json!({}))
                .send()
                .await?;
            if !response.status().is_success() {
                return Err(format!("session end failed: {}", response.status()).into());
            }
            fs::remove_file(path(id)?)?;
            println!(
                "{}",
                serde_json::json!({"ended":id,"room_id":record.room_id})
            );
        }
        SessionAction::Heartbeat { id } => {
            let record = load(id, url)?;
            heartbeat(url, &record).await?;
        }
    }
    Ok(())
}

async fn heartbeat(url: &str, record: &SessionRecord) -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = format!("/social/v1/sessions/{}/heartbeat", record.id);
    let response = reqwest::Client::new()
        .post(http_url(url, &endpoint)?)
        .header("x-cowchat-key", &record.session_token)
        .json(&serde_json::json!({}))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(format!("session heartbeat failed: {}", response.status()).into());
    }
    Ok(())
}
