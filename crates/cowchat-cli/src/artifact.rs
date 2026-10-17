use super::{connect, load_key, resolve_room_id, Cli};
use clap::{Args as ClapArgs, Subcommand};
use reqwest::{Method, Url};
use serde_json::{json, Value};
use std::{io::Read, path::PathBuf, time::Duration};
use uuid::Uuid;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const MAX_VIEW_BYTES: u64 = 64 * 1024;
const MAX_DATA_BYTES: u64 = 256 * 1024;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(ClapArgs)]
pub struct Args {
    /// Reuse this UUID when retrying the same publication or data update.
    #[arg(long, global = true)]
    request_id: Option<Uuid>,
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Publish selected data at an unlisted read-only URL. Does not publish room history.
    Publish {
        room: String,
        #[arg(long)]
        title: String,
        /// Declarative Gallop/OpenUI layout with $data bindings.
        #[arg(long)]
        view: PathBuf,
        /// Initial JSON object. Omit to retain existing data when changing the layout.
        #[arg(long)]
        data: Option<PathBuf>,
        /// Required when replacing an existing artifact's layout or complete data.
        #[arg(long)]
        if_revision: Option<u64>,
    },
    /// Read the current layout, data, revision, and share URL.
    Get { room: String },
    /// Merge selected JSON keys. Null removes a key; arrays replace the whole array.
    Update {
        room: String,
        #[arg(long)]
        json: String,
        #[arg(long)]
        if_revision: Option<u64>,
    },
    /// Print the room's existing artifact URL.
    Link { room: String },
    /// Revoke the existing share URL and return a new one.
    RotateLink { room: String },
    /// Remove the artifact and revoke its URL.
    Unpublish { room: String },
}

fn http_base(raw: Option<&str>) -> Result<Url> {
    let mut url = Url::parse(raw.ok_or("artifacts need --url wss://<host>/ws")?)?;
    let scheme = match url.scheme() {
        "wss" => "https",
        "ws" => "http",
        _ => return Err("artifacts need a ws:// or wss:// server URL".into()),
    };
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "artifact server URLs cannot contain credentials, queries, or fragments".into(),
        );
    }
    url.set_scheme(scheme)
        .map_err(|_| "invalid server scheme")?;
    let path = url.path().trim_end_matches('/');
    let path = path.strip_suffix("/ws").unwrap_or(path).to_owned();
    url.set_path(&format!("{path}/"));
    Ok(url)
}

fn endpoint(base: &Url, segments: &[&str]) -> Result<Url> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| "invalid artifact server URL")?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

fn read_bounded(path: &PathBuf, limit: u64) -> Result<String> {
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > limit {
        return Err(format!("{} exceeds {limit} bytes", path.display()).into());
    }
    Ok(text)
}

fn data_object(text: &str) -> Result<Value> {
    if text.len() as u64 > MAX_DATA_BYTES {
        return Err("artifact data exceeds 256 KiB".into());
    }
    let value: Value = serde_json::from_str(text)?;
    if !value.is_object() {
        return Err("artifact data must be a JSON object".into());
    }
    Ok(value)
}

async fn request(method: Method, url: Url, key: &str, body: Option<Value>) -> Result<Value> {
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()?;
    let mut request = http.request(method, url).header("x-cowchat-key", key);
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&body)?);
    }
    let mut response = request.send().await?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err("artifact response exceeds 1 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let message = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| "artifact request failed".to_owned());
        return Err(format!("{status}: {message}").into());
    }
    if bytes.is_empty() {
        return Ok(json!({"unpublished": true}));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

pub async fn run(cli: &Cli, args: &Args) -> Result<()> {
    let base = http_base(cli.url.as_deref())?;
    let request_id = args.request_id.unwrap_or_else(Uuid::new_v4).to_string();
    let (room, method, suffix, body) = match &args.action {
        Action::Publish {
            room,
            title,
            view,
            data,
            if_revision,
        } => {
            let view = read_bounded(view, MAX_VIEW_BYTES)?;
            let mut body = json!({"title":title,"view":view,"request_id":request_id});
            if let Some(path) = data {
                body["data"] = data_object(&read_bounded(path, MAX_DATA_BYTES)?)?;
            }
            if let Some(revision) = if_revision {
                body["expected_revision"] = json!(revision);
            }
            (room, Method::PUT, None, Some(body))
        }
        Action::Update {
            room,
            json: patch,
            if_revision,
        } => {
            let mut body = json!({"patch":data_object(patch)?,"request_id":request_id});
            if let Some(revision) = if_revision {
                body["expected_revision"] = json!(revision);
            }
            (room, Method::PATCH, None, Some(body))
        }
        Action::Get { room } | Action::Link { room } => (room, Method::GET, None, None),
        Action::RotateLink { room } => (
            room,
            Method::POST,
            Some("rotate"),
            Some(json!({"request_id":request_id})),
        ),
        Action::Unpublish { room } => (room, Method::DELETE, None, None),
    };
    let client = connect(cli).await?;
    let room_id = resolve_room_id(&client, room).await?;
    let mut segments = vec!["api", "rooms", &room_id, "artifact"];
    if let Some(suffix) = suffix {
        segments.push(suffix);
    }
    let mut result = request(
        method,
        endpoint(&base, &segments)?,
        &load_key(&cli.key),
        body,
    )
    .await?;
    if let Some(id) = result.get("public_id").and_then(Value::as_str) {
        let id = Uuid::parse_str(id)?.to_string();
        let url = endpoint(&base, &["a", &id])?;
        if matches!(args.action, Action::Link { .. }) {
            println!("{url}");
            return Ok(());
        }
        result["url"] = json!(url.as_str());
    }
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_urls_preserve_prefix_and_encode_room_segment() {
        let base = http_base(Some("wss://example.test/chat/ws")).unwrap();
        assert_eq!(
            endpoint(&base, &["api", "rooms", "a/b", "artifact"])
                .unwrap()
                .as_str(),
            "https://example.test/chat/api/rooms/a%2Fb/artifact"
        );
        for raw in [
            "wss://user:secret@example.test/ws",
            "wss://example.test/ws?key=secret",
            "https://example.test/ws",
        ] {
            assert!(http_base(Some(raw)).is_err());
        }
    }

    #[test]
    fn patches_accept_nested_deletions_but_reject_root_arrays() {
        assert!(data_object(r#"{"prs":{"core-618":null}}"#).is_ok());
        assert!(data_object("[]").is_err());
        assert!(data_object("null").is_err());
    }
}
