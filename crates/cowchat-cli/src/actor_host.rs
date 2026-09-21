//! Local wake receiver. Execution is a configured program, never wake-supplied code.
use super::*;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use std::{net::SocketAddr, process::Stdio, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Notify,
};

#[derive(clap::Args)]
pub(crate) struct ActorHostArgs {
    /// Room name or UUID. Create an encrypted room and set COWCHAT_ROOM_KEY first.
    pub room: String,
    #[arg(long, default_value = "127.0.0.1:9230")]
    pub listen: SocketAddr,
    #[arg(long, default_value = "addressed", value_parser = ["always", "addressed", "listen"])]
    pub mode: String,
    /// Program receiving one ActorWork JSON on stdin and returning reply text on stdout.
    /// This program is trusted local configuration, not a message-supplied command.
    #[arg(last = true, required = true)]
    pub command: Vec<String>,
}

struct WakeState {
    room: String,
    subscription: String,
    secret: String,
    notify: Arc<Notify>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wake {
    #[serde(rename = "type")]
    kind: String,
    room_id: String,
    subscription_id: String,
}

fn authenticated(headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
    let field = |name| headers.get(name).and_then(|v| v.to_str().ok());
    let (Some(id), Some(timestamp), Some(signature)) = (
        field("webhook-id"),
        field("webhook-timestamp"),
        field("webhook-signature"),
    ) else {
        return false;
    };
    let Ok(time) = timestamp.parse::<i64>() else {
        return false;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    if now.abs_diff(time) > 300 {
        return false;
    }
    let Some(encoded) = signature.strip_prefix("v1,") else {
        return false;
    };
    let Ok(bytes) = STANDARD.decode(encoded) else {
        return false;
    };
    let mut mac =
        Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key");
    mac.update(format!("{id}.{timestamp}.").as_bytes());
    mac.update(body);
    mac.verify_slice(&bytes).is_ok()
}

async fn wake(
    State(state): State<Arc<WakeState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> StatusCode {
    if !authenticated(&headers, &body, &state.secret) {
        return StatusCode::UNAUTHORIZED;
    }
    let Ok(wake) = serde_json::from_slice::<Wake>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    if wake.kind != "cowchat.actor.wake"
        || wake.room_id != state.room
        || wake.subscription_id != state.subscription
    {
        return StatusCode::FORBIDDEN;
    }
    state.notify.notify_one();
    StatusCode::ACCEPTED
}

async fn execute(command: &[String], work: cowchat_core::ActorWork) -> Result<String, ClientError> {
    let mut child = tokio::process::Command::new(&command[0])
        .args(&command[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_remove("COWCHAT_ROOM_KEY")
        .env_remove("COWCHAT_WAKE_SECRET")
        .env_remove("COWCHAT_KEY")
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let input = serde_json::to_vec(&work)?;
    let result = tokio::time::timeout(std::time::Duration::from_secs(240), async {
        let write = async {
            stdin.write_all(&input).await?;
            drop(stdin);
            Ok::<(), std::io::Error>(())
        };
        let read = async {
            let mut output = Vec::new();
            (&mut stdout)
                .take(1024 * 1024 + 1)
                .read_to_end(&mut output)
                .await?;
            if output.len() > 1024 * 1024 {
                return Err(std::io::Error::other("actor output exceeds 1 MiB"));
            }
            Ok(output)
        };
        let (_, output) = tokio::try_join!(write, read)?;
        let status = child.wait().await?;
        if !status.success() {
            return Err(std::io::Error::other("actor program failed"));
        }
        String::from_utf8(output).map_err(|_| std::io::Error::other("actor reply is not UTF-8"))
    })
    .await
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "actor program exceeded 240 seconds",
        )
    })??;
    Ok(result)
}

pub(crate) async fn run(cli: &Cli, args: &ActorHostArgs) -> Result<(), Box<dyn std::error::Error>> {
    if resolve_agent_id(cli).is_none() {
        return Err("actor-host requires --agent-id".into());
    }
    if !args.listen.ip().is_loopback() || args.listen.port() == 0 {
        return Err("local actor-host requires a fixed loopback listen address".into());
    }
    let secret = env_non_empty("COWCHAT_WAKE_SECRET")
        .ok_or("set COWCHAT_WAKE_SECRET for signed wake requests")?;
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    let client = connect(cli).await?;
    let room = resolve_room_id(&client, &args.room).await?;
    client.join_room(&room).await?;
    let mode = match args.mode.as_str() {
        "always" => cowchat_core::WakeMode::Always,
        "listen" => cowchat_core::WakeMode::Listen,
        _ => cowchat_core::WakeMode::Addressed,
    };
    let subscription = client
        .subscribe_actor(cowchat_core::SubscribeActorPayload {
            room_id: room.clone(),
            webhook_url: format!("http://{}/wake", args.listen),
            secret: secret.clone(),
            mode,
        })
        .await?;
    drop(client);
    let notify = Arc::new(Notify::new());
    let state = Arc::new(WakeState {
        room: room.clone(),
        subscription: subscription.clone(),
        secret,
        notify: notify.clone(),
    });
    let app = Router::new()
        .route("/wake", post(wake))
        .layer(axum::extract::DefaultBodyLimit::max(4096))
        .with_state(state);
    let mut http = tokio::spawn(async move { axum::serve(listener, app).await });
    println!(
        "{}",
        serde_json::json!({"room_id":room, "subscription_id":subscription, "listen":args.listen.to_string(), "status":"ready"})
    );
    notify.notify_one(); // recover queued work immediately after a receiver restart
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => { http.abort(); return Ok(()); }
            result = &mut http => { return Err(format!("wake receiver stopped: {result:?}").into()); }
            _ = notify.notified() => {}
        }
        let result = async {
            let client = connect(cli).await?;
            client.join_room(&room).await?;
            loop {
                let mut failed_work = None;
                let failed = &mut failed_work;
                let outcome = client.process_actor_work(&subscription, |work| async move {
                    for attempt in 1..=3 {
                        match execute(&args.command, work.clone()).await {
                            Ok(reply) => return Ok(reply),
                            Err(error) => {
                                eprintln!("actor work {} execution attempt {attempt}/3 failed: {error}", work.work_id);
                                if attempt == 3 { *failed = Some(work.work_id.clone()); return Err(error); }
                            }
                        }
                    }
                    unreachable!()
                }).await;
                if let Some(work_id) = failed_work {
                    client.complete_actor_work(&subscription, &work_id, cowchat_core::ActorWorkOutcome::Failed).await?;
                    eprintln!("actor work {work_id} marked FAILED after 3 execution failures; continuing with later work");
                    continue;
                }
                if !outcome? { break; }
            }
            Ok::<(), Box<dyn std::error::Error>>(())
        }
        .await;
        if let Err(error) = result {
            eprintln!("actor work pending; next wake will retry: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wake_signature_rejects_modified_body_and_old_requests() {
        let body = b"{}";
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();
        let mut headers = HeaderMap::new();
        headers.insert("webhook-id", "id".parse().unwrap());
        headers.insert("webhook-timestamp", now.parse().unwrap());
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"secret").unwrap();
        mac.update(format!("id.{now}.{{}}").as_bytes());
        headers.insert(
            "webhook-signature",
            format!("v1,{}", STANDARD.encode(mac.finalize().into_bytes()))
                .parse()
                .unwrap(),
        );
        assert!(authenticated(&headers, body, "secret"));
        assert!(!authenticated(&headers, b"changed", "secret"));
        assert!(!authenticated(&headers, body, "wrong"));
        headers.insert("webhook-timestamp", "0".parse().unwrap());
        assert!(!authenticated(&headers, body, "secret"));
    }
}
