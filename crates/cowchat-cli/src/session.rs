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

fn validate_id(id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let suffix = id.strip_prefix("session:").ok_or("invalid session ID")?;
    uuid::Uuid::parse_str(suffix)?;
    Ok(())
}

pub(crate) fn load_bound(
    id: &str,
    url: &str,
) -> Result<(SessionRecord, PathBuf), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::var("COWCHAT_SESSION_FILE")
            .map_err(|_| "COWCHAT_SESSION_FILE must be set by this session's hook")?,
    );
    let meta = fs::metadata(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err("session token file permissions are too broad".into());
        }
    }
    let record: SessionRecord = serde_json::from_slice(&fs::read(&path)?)?;
    if record.id != id || record.url != url || !record.session_token.starts_with("csoc_") {
        return Err("session token does not match this ID and server".into());
    }
    Ok((record, path))
}

fn save(record: &SessionRecord) -> Result<PathBuf, Box<dyn std::error::Error>> {
    validate_id(&record.id)?;
    let target = default_data_dir()
        .join("sessions")
        .join(format!("{}.json", uuid::Uuid::new_v4()));
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
        fs::rename(&temporary, &target)?;
    }
    #[cfg(not(unix))]
    {
        return Err("hosted session files require Unix permissions".into());
    }
    Ok(target)
}

pub(crate) fn http_url(url: &str, route: &str) -> Result<reqwest::Url, Box<dyn std::error::Error>> {
    let mut parsed = reqwest::Url::parse(url)?;
    if parsed.path() != "/ws"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("expected a hosted /ws URL".into());
    }
    let scheme = match parsed.scheme() {
        "wss" => "https",
        "ws" if parsed.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        }) =>
        {
            "http"
        }
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
            validate_id(&id)?;
            if std::env::var_os("COWCHAT_SESSION_FILE").is_some() {
                let (saved, bound_path) = load_bound(&id, url)?;
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
                    serde_json::json!({"session_id":saved.id,"room_id":saved.room_id,"room_name":saved.room_name,"session_file":bound_path})
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
            let saved_path = save(&record)?;
            println!(
                "{}",
                serde_json::json!({"session_id":id,"room_id":record.room_id,"room_name":record.room_name,"session_file":saved_path})
            );
        }
        SessionAction::End { id } => {
            let (record, bound_path) = load_bound(id, url)?;
            let endpoint = format!("/social/v1/sessions/{id}/end");
            let response = reqwest::Client::new()
                .post(http_url(url, &endpoint)?)
                .header("x-cowchat-key", &record.session_token)
                .json(&serde_json::json!({}))
                .send()
                .await?;
            if !response.status().is_success() {
                if response.status() == reqwest::StatusCode::BAD_REQUEST
                    || response.status() == reqwest::StatusCode::UNAUTHORIZED
                {
                    fs::remove_file(&bound_path)?;
                }
                return Err(format!("session end failed: {}", response.status()).into());
            }
            fs::remove_file(bound_path)?;
            println!(
                "{}",
                serde_json::json!({"ended":id,"room_id":record.room_id})
            );
        }
        SessionAction::Heartbeat { id } => {
            let (record, _) = load_bound(id, url)?;
            heartbeat(url, &record).await?;
        }
    }
    Ok(())
}

pub(crate) async fn heartbeat(
    url: &str,
    record: &SessionRecord,
) -> Result<(), Box<dyn std::error::Error>> {
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

pub(crate) async fn approved_room_ids(
    id: &str,
    url: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let (record, _) = load_bound(id, url)?;
    let route = format!("/social/v1/sessions/{id}/rooms");
    let response = reqwest::Client::new()
        .get(http_url(url, &route)?)
        .header("x-cowchat-key", &record.session_token)
        .send()
        .await?
        .error_for_status()?;
    let body: serde_json::Value = response.json().await?;
    let rooms = body["rooms"]
        .as_array()
        .ok_or("missing approved rooms")?
        .iter()
        .map(|room| {
            room["room_id"]
                .as_str()
                .map(str::to_owned)
                .ok_or("missing room ID")
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rooms)
}

pub(crate) async fn room_id(
    id: &str,
    url: &str,
    token: &str,
    room: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let route = format!("/social/v1/sessions/{id}/rooms");
    let response = reqwest::Client::new()
        .get(http_url(url, &route)?)
        .header("x-cowchat-key", token)
        .send()
        .await?
        .error_for_status()?;
    let body: serde_json::Value = response.json().await?;
    let matches = body["rooms"]
        .as_array()
        .ok_or("missing approved rooms")?
        .iter()
        .filter(|item| item["room_id"] == room || item["name"] == room)
        .filter_map(|item| item["room_id"].as_str())
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [only] => Ok((*only).to_owned()),
        [] => Err("room is not approved for this session".into()),
        _ => Err("room name is ambiguous; use its ID".into()),
    }
}

pub(crate) async fn send(
    cli: &Cli,
    id: &str,
    room: &str,
    content: &str,
    reply_to: Option<&str>,
    mentions: &[String],
    metadata: serde_json::Value,
) -> Result<cowchat_core::ChatMessage, Box<dyn std::error::Error>> {
    let url = cli.url.as_deref().ok_or("hosted sessions require --url")?;
    http_url(url, "/social/v1/sessions")?;
    let (record, _) = load_bound(id, url)?;
    if cli.name != "cli" && cli.name != record.name {
        return Err("--name does not match the saved session".into());
    }
    let room_id = room_id(id, url, &record.session_token, room).await?;
    let route = format!("/social/v1/sessions/{id}/messages");
    let response = reqwest::Client::new()
        .post(http_url(url, &route)?)
        .header("x-cowchat-key", &record.session_token)
        .json(&serde_json::json!({"room_id":room_id,"content":content,"reply_to":reply_to,"mentions":mentions,"metadata":metadata}))
        .send().await?.error_for_status()?;
    Ok(response.json().await?)
}

pub(crate) async fn presence(
    cli: &Cli,
    id: &str,
    status: &str,
    detail: Option<&str>,
    progress: Option<u8>,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = cli.url.as_deref().ok_or("hosted sessions require --url")?;
    http_url(url, "/social/v1/sessions")?;
    let (record, _) = load_bound(id, url)?;
    if cli.name != "cli" && cli.name != record.name {
        return Err("--name does not match the saved session".into());
    }
    let route = format!("/social/v1/sessions/{id}/presence");
    reqwest::Client::new()
        .post(http_url(url, &route)?)
        .header("x-cowchat-key", &record.session_token)
        .json(&serde_json::json!({"status":status,"status_detail":detail,"progress":progress}))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::http_url;

    #[test]
    fn session_http_never_sends_tokens_to_remote_plaintext() {
        assert!(http_url("ws://chat.example/ws", "/social/v1/sessions").is_err());
        assert!(http_url("ws://127.0.0.1:9229/ws", "/social/v1/sessions").is_ok());
        assert!(http_url("wss://chat.example/ws", "/social/v1/sessions").is_ok());
    }
}
