//! A host-side heartbeat supervisor. Waiting state is reported by the host, not guessed.
use super::{connect, resolve_agent_id, resolve_room_id, Cli, CowchatClient, FrameType};
use clap::{Args as ClapArgs, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    io::{IsTerminal, Read},
    path::{Path, PathBuf},
    time::Duration,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    action: Action,
}
#[derive(Subcommand)]
enum Action {
    List {
        room: String,
    },
    Status {
        room: String,
        resource: String,
    },
    Acquire {
        room: String,
        resource: String,
    },
    Heartbeat {
        room: String,
        resource: String,
        #[arg(long)]
        generation: i64,
        #[arg(long, value_enum)]
        state: Option<State>,
        #[arg(long)]
        detail: Option<String>,
        #[arg(long)]
        progress: Option<String>,
    },
    Release {
        room: String,
        resource: String,
        #[arg(long)]
        generation: i64,
    },
    Validate {
        room: String,
        resource: String,
        #[arg(long)]
        generation: i64,
        /// Expected holder. Defaults to this agent ID.
        #[arg(long)]
        holder: Option<String>,
    },
    /// Renew every 60 seconds while COMMAND runs. Host writes waiting/progress state to --state-file.
    Run {
        room: String,
        resource: String,
        #[arg(long)]
        state_file: Option<PathBuf>,
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Record this agent's latest verdict for an exact candidate commit.
    Ready {
        room: String,
        resource: String,
        #[arg(long)]
        commit: String,
        #[arg(long, value_enum)]
        verdict: Verdict,
        #[arg(long, default_value = "")]
        reason: String,
    },
    Readiness {
        room: String,
        resource: String,
        #[arg(long)]
        commit: String,
    },
}
#[derive(Clone, Copy, ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Working,
    WaitingForHuman,
    WaitingForDependency,
}
#[derive(Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
enum Verdict {
    Ready,
    Blocked,
    Withdrawn,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostState {
    state: Option<State>,
    detail: Option<String>,
    progress: Option<String>,
}
#[derive(Deserialize)]
struct Grant {
    #[serde(rename = "room_id")]
    room: String,
    name: String,
    generation: i64,
}
async fn request(client: &CowchatClient, room: &str, operation: Value) -> Result<Value> {
    Ok(client
        .request(
            FrameType::RoomCoordination,
            json!({"room_id":room,"operation":operation}),
        )
        .await?
        .payload)
}
async fn acquire(client: &CowchatClient, room: &str, name: &str) -> Result<Value> {
    let room = resolve_room_id(client, room).await?;
    client.join_room(&room).await?;
    let current = request(client, &room, json!({"action":"get","name":name})).await?;
    let generation = current["lease"]["generation"].as_i64().unwrap_or(0);
    request(
        client,
        &room,
        json!({"action":"acquire","name":name,"expected_generation":generation}),
    )
    .await
}
fn operation(grant: &Grant, action: &str) -> Value {
    json!({"action":action,"name":grant.name,"generation":grant.generation})
}
async fn heartbeat(client: &CowchatClient, grant: &Grant, state: HostState) -> Result<Value> {
    let mut op = operation(grant, "heartbeat");
    if let Some(s) = state.state {
        op["state"] = serde_json::to_value(s)?;
    }
    if let Some(s) = state.detail {
        op["detail"] = s.into();
    }
    if let Some(s) = state.progress {
        op["progress"] = s.into();
    }
    request(client, &grant.room, op).await
}
fn host_state(path: Option<&Path>) -> Result<HostState> {
    let Some(path) = path else {
        return Ok(HostState::default());
    };
    use std::os::unix::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err("Host state must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(8193).read_to_end(&mut bytes)?;
    if bytes.len() > 8192 {
        return Err("Host state file too large".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
async fn signal() -> Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate())?;
    let mut hup = signal(SignalKind::hangup())?;
    tokio::select! {r=tokio::signal::ctrl_c()=>r?,_=term.recv()=>{},_=hup.recv()=>{}};
    Ok(())
}
async fn supervise(
    client: &CowchatClient,
    grant: &Grant,
    state_file: Option<&Path>,
    command: &[String],
) -> Result<i32> {
    // Check host state before starting side effects and report it immediately.
    heartbeat(client, grant, host_state(state_file)?).await?;
    let mut cmd = tokio::process::Command::new(&command[0]);
    cmd.args(&command[1..]).kill_on_drop(true);
    cmd.env("COWCHAT_LEASE_ROOM", &grant.room)
        .env("COWCHAT_LEASE_RESOURCE", &grant.name)
        .env("COWCHAT_LEASE_GENERATION", grant.generation.to_string());
    let isolated = !std::io::stdin().is_terminal();
    if isolated {
        cmd.process_group(0);
    }
    if let Some(path) = state_file {
        cmd.env("COWCHAT_LEASE_STATE_FILE", path);
    }
    let mut child = cmd.spawn()?;
    let mut timer = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(60),
        Duration::from_secs(60),
    );
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let stop = signal();
    tokio::pin!(stop);
    let result = loop {
        tokio::select! {
            status=child.wait()=>break status.map(|s|s.code().unwrap_or(1)).map_err(Into::into),
            result=&mut stop=>break result.and(Err("Lease command interrupted".into())),
            _=timer.tick()=>{
                let state=match host_state(state_file){Ok(s)=>s,Err(e)=>break Err(e)};
                if heartbeat(client,grant,state).await.is_err(){break Err("Lease heartbeat failed; stopping command. Validate ownership before retrying.".into());}
            }
        }
    };
    if isolated {
        if let Some(pid) = child.id() {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}
pub async fn run(cli: &Cli, args: &Args) -> Result<()> {
    if matches!(
        args.action,
        Action::Acquire { .. }
            | Action::Heartbeat { .. }
            | Action::Release { .. }
            | Action::Run { .. }
            | Action::Ready { .. }
    ) && resolve_agent_id(cli).is_none()
    {
        return Err("Room coordination requires a stable --agent-id or COWCHAT_AGENT_ID".into());
    }
    let client = connect(cli).await?;
    let result = match &args.action {
        Action::List { room } => {
            let id = resolve_room_id(&client, room).await?;
            request(&client, &id, json!({"action":"list"})).await?
        }
        Action::Status { room, resource } => {
            let id = resolve_room_id(&client, room).await?;
            request(&client, &id, json!({"action":"get","name":resource})).await?
        }
        Action::Acquire { room, resource } => acquire(&client, room, resource).await?,
        Action::Run {
            room,
            resource,
            state_file,
            command,
        } => {
            let acquired = acquire(&client, room, resource).await?;
            let grant: Grant = serde_json::from_value(acquired["lease"].clone())?;
            let outcome = supervise(&client, &grant, state_file.as_deref(), command).await;
            let released = request(&client, &grant.room, operation(&grant, "release")).await;
            let code = outcome?;
            released?;
            if code != 0 {
                std::process::exit(code);
            }
            return Ok(());
        }
        Action::Heartbeat {
            room,
            resource,
            generation,
            state,
            detail,
            progress,
        } => {
            let id = resolve_room_id(&client, room).await?;
            client.join_room(&id).await?;
            heartbeat(
                &client,
                &Grant {
                    room: id,
                    name: resource.clone(),
                    generation: *generation,
                },
                HostState {
                    state: *state,
                    detail: detail.clone(),
                    progress: progress.clone(),
                },
            )
            .await?
        }
        Action::Release {
            room,
            resource,
            generation,
        } => {
            let id = resolve_room_id(&client, room).await?;
            client.join_room(&id).await?;
            request(
                &client,
                &id,
                json!({"action":"release","name":resource,"generation":generation}),
            )
            .await?
        }
        Action::Validate {
            room,
            resource,
            generation,
            holder,
        } => {
            let id = resolve_room_id(&client, room).await?;
            let holder = holder
                .clone()
                .or_else(|| resolve_agent_id(cli))
                .ok_or("Pass --holder or use a stable --agent-id")?;
            request(&client, &id, json!({"action":"validate","name":resource,"generation":generation,"holder":holder})).await?
        }
        Action::Ready {
            room,
            resource,
            commit,
            verdict,
            reason,
        } => {
            let id = resolve_room_id(&client, room).await?;
            client.join_room(&id).await?;
            request(&client,&id,json!({"action":"readiness","name":resource,"commit":commit,"verdict":verdict,"reason":reason})).await?
        }
        Action::Readiness {
            room,
            resource,
            commit,
        } => {
            let id = resolve_room_id(&client, room).await?;
            request(
                &client,
                &id,
                json!({"action":"readiness_list","name":resource,"commit":commit}),
            )
            .await?
        }
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_host_state_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");
        assert!(host_state(Some(&path)).is_err());
        std::fs::write(
            &path,
            r#"{"state":"waiting_for_human","detail":"Approve deploy"}"#,
        )
        .unwrap();
        assert!(matches!(
            host_state(Some(&path)).unwrap().state,
            Some(State::WaitingForHuman)
        ));
        std::fs::write(&path, r#"{"state":"imagined"}"#).unwrap();
        assert!(host_state(Some(&path)).is_err());
    }
}
