use clap::{Parser, Subcommand, ValueEnum};
use cowchat_client::{ClientError, CowchatClient};
use cowchat_core::{ChatMessage, ErrorCode, FrameType};
use std::io::Write;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};

mod lantern;

/// `metadata.kind` marking the last message of a conversation. `send --end` sets
/// it; `wait` exits 3 on receiving one so a reply-then-wait loop terminates.
const KIND_CONVERSATION_END: &str = "conversation_end";

fn render_export(
    messages: &[ChatMessage],
    format: ExportFormat,
    include_thinking: bool,
    room_label: &str,
) -> String {
    let mut out = String::new();
    let filtered: Vec<&ChatMessage> = messages
        .iter()
        .filter(|m| {
            let kind = m.metadata.get("type").and_then(|v| v.as_str());
            if kind == Some("system") {
                return false; // never include system rows in exports
            }
            if !include_thinking && kind == Some("thinking") {
                return false;
            }
            true
        })
        .collect();

    match format {
        ExportFormat::Md => {
            out.push_str(&format!("# Room: {}\n\n", room_label));
            if let (Some(first), Some(last)) = (filtered.first(), filtered.last()) {
                out.push_str(&format!(
                    "_{} entries from seq {} to {} ({} → {})_\n\n",
                    filtered.len(),
                    first.seq,
                    last.seq,
                    first.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
                    last.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
                ));
            }
            for m in &filtered {
                let kind = m.metadata.get("type").and_then(|v| v.as_str());
                let header = match kind {
                    Some("thinking") => format!(
                        "### 💭 {} · seq {} · {}",
                        m.agent_name,
                        m.seq,
                        m.timestamp.format("%H:%M:%S")
                    ),
                    _ => format!(
                        "### {} · seq {} · {}",
                        m.agent_name,
                        m.seq,
                        m.timestamp.format("%H:%M:%S")
                    ),
                };
                out.push_str(&header);
                out.push_str("\n\n");
                out.push_str(&m.content);
                if !m.content.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
            }
        }
        ExportFormat::Json => {
            for m in &filtered {
                out.push_str(&serde_json::to_string(m).unwrap_or_default());
                out.push('\n');
            }
        }
        ExportFormat::Txt => {
            for m in &filtered {
                let kind = m.metadata.get("type").and_then(|v| v.as_str());
                let tag = if kind == Some("thinking") {
                    "(thinking) "
                } else {
                    ""
                };
                out.push_str(&format!(
                    "[{}] #{} {}{}: {}\n",
                    m.timestamp.format("%H:%M:%S"),
                    m.seq,
                    tag,
                    m.agent_name,
                    m.content
                ));
                out.push('\n');
            }
        }
    }
    out
}

fn format_message(msg: &ChatMessage) -> String {
    let ts = msg.timestamp.format("%H:%M:%S");
    let is_system = msg.metadata.get("type").and_then(|v| v.as_str()) == Some("system");
    if is_system {
        format!(
            "[{}] #{} * {} {} *",
            ts, msg.seq, msg.agent_name, msg.content
        )
    } else {
        format!("[{}] #{} {}: {}", ts, msg.seq, msg.agent_name, msg.content)
    }
}

#[derive(Parser)]
#[command(
    name = "cowchat",
    version,
    about = "Cowchat - Agent-to-agent chat infrastructure"
)]
struct Cli {
    /// Unix socket path to connect to
    #[arg(long, global = true, default_value = default_socket_path())]
    socket: PathBuf,

    /// Use TCP instead of Unix socket
    #[arg(long, global = true)]
    tcp: Option<String>,

    /// Connect over WebSocket to a remote server, e.g.
    /// wss://your-server.example/ws. Takes precedence over --tcp/--socket.
    #[arg(long, global = true)]
    url: Option<String>,

    /// API key for authenticated remote servers (local connections are keyless)
    #[arg(long, global = true)]
    key: Option<String>,

    /// Pre-shared key for end-to-end encrypted rooms. Overrides the
    /// COWCHAT_ROOM_KEY environment variable.
    /// Content is encrypted before send and decrypted after receive, per-room.
    #[arg(long, global = true)]
    room_key: Option<String>,

    /// Agent name for this CLI session
    #[arg(long, global = true, default_value = "cli")]
    name: String,

    /// Stable agent id for this session. Pass a consistent value across calls so
    /// the server treats separate send/wait invocations as one logical agent.
    /// May also be supplied through COWCHAT_AGENT_ID. Named agent workflows fail
    /// closed when neither source is set.
    #[arg(long, global = true)]
    agent_id: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Send a message to a room
    Send {
        /// Room ID or name
        room: String,
        /// Message content
        message: String,
        /// Reply to a specific message ID
        #[arg(long)]
        reply_to: Option<String>,
        /// Tag the message with a `kind` (stored in `metadata.kind`). Free-form,
        /// but conventions: `review_request`, `verdict`, `checkpoint`, `fyi`.
        /// Peers can filter on these via `wait --only-kind` / `history --kind`.
        #[arg(long)]
        kind: Option<String>,
        /// End the conversation: tags the message `kind=conversation_end`. A peer
        /// running `wait` surfaces this message, then exits 3 so its reply-then-wait
        /// loop terminates cleanly instead of blocking for another turn.
        #[arg(long, conflicts_with = "kind")]
        end: bool,
    },

    /// Post a "thinking out loud" pulse to a room (persisted to history,
    /// broadcast as a `thinking` event, does NOT advance the turn token,
    /// does NOT wake peers' `wait`).
    Thinking {
        /// Room ID or name
        room: String,
        /// Thought / status content (keep it short)
        content: String,
    },

    /// Room management
    Rooms {
        #[command(subcommand)]
        action: RoomAction,
    },

    /// List connected agents
    Agents {
        /// Filter by room ID
        #[arg(long)]
        room: Option<String>,
    },

    /// View message history
    History {
        /// Room ID
        room: String,
        /// Number of messages
        #[arg(long, default_value = "50")]
        limit: u32,
        /// Stream new messages (like tail -f)
        #[arg(long)]
        follow: bool,
        /// Only return messages after this message ID
        #[arg(long)]
        since: Option<String>,
        /// Only return messages with seq strictly greater than this value
        #[arg(long)]
        since_seq: Option<i64>,
        /// Only return messages tagged with this `metadata.kind`
        /// (e.g. `--kind verdict`).
        #[arg(long)]
        kind: Option<String>,
        /// Write to file instead of stdout
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
    },

    /// Wait for a new message in a room (blocks until one arrives)
    Wait {
        /// Room ID or name
        room: String,
        /// Timeout in seconds (0 = wait forever). In --loop mode this is the
        /// per-iteration budget; outer wall-clock is unbounded.
        #[arg(long, default_value = "60")]
        timeout: u64,
        /// Output as JSON. Default. Pass --text for human-readable output.
        #[arg(long, conflicts_with = "text")]
        json: bool,
        /// Output as human-readable text instead of JSON.
        #[arg(long)]
        text: bool,
        /// Also catch up: return the oldest chat message with seq > this value
        /// if one already exists in history, else block for a new message.
        /// Use this to safely resume after a prior wait — pass the seq of the
        /// last message you saw. Accepts an integer, or `tip`/`auto` to resolve
        /// to the room's current tip on start.
        #[arg(long)]
        since_seq: Option<String>,
        /// Stay in wait indefinitely: re-poll on internal timeout and reconnect
        /// transport failures with bounded backoff (tracking the bookmark) until
        /// a real chat message arrives. Pairs with --since-seq so messages that
        /// land between iterations are never missed. With this flag the single
        /// CLI invocation replaces the "re-run wait on timeout" discipline — the
        /// active agent turn makes one call and gets one message back. After the
        /// caller handles and replies, it must invoke this returning wait again;
        /// Cowchat cannot by itself resume a task whose turn has already ended.
        #[arg(long = "loop")]
        loop_: bool,
        /// Keep streaming messages until interrupted or a conversation_end is
        /// received. Intended for human-watched observation or an always-on
        /// consumer; it does not return each message to a turn-based agent.
        #[arg(long)]
        follow: bool,
        /// Bound the total wall-clock of a `--loop` wait (seconds). On expiry the
        /// command exits 2 (distinct from 0=message, 1=error) and prints the seq to
        /// resume from, so a stalled turn returns control instead of hanging forever.
        /// 0 = unbounded (default). Without `--loop`, `--timeout` already bounds the
        /// single wait, so this has no effect there.
        #[arg(long, default_value = "0")]
        idle_timeout: u64,
        /// Seconds between liveness heartbeats printed to stderr while blocked.
        /// Tool wrappers that kill silent processes see this and let the wait
        /// continue. 0 disables. Default 30.
        #[arg(long, default_value = "30")]
        heartbeat_secs: u64,
        /// Only wake on messages from this `--name` (peer filter).
        #[arg(long)]
        only_from: Option<String>,
        /// Skip messages from this `--name` (in addition to your own).
        #[arg(long)]
        not_from: Option<String>,
        /// Only wake on messages tagged with this `metadata.kind`.
        /// E.g. `--only-kind review_request`.
        #[arg(long)]
        only_kind: Option<String>,
        /// Print peer `thinking` pulses to stderr while blocked, for live
        /// visibility during long runs. Does NOT wake the wait — it still only
        /// returns on a real chat message, preserving turn-taking. Content is
        /// decrypted if a room key is set.
        #[arg(long)]
        show_thinking: bool,
        /// Write result to file instead of stdout. Useful when tool wrappers
        /// truncate large stdout payloads.
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        /// Wake on the next message, then emit EVERY unread message through the
        /// room's current tip (one JSON object per line), not just the one that
        /// woke the wait. Drain before composing so a correction that landed
        /// while you were thinking isn't answered a turn late. The cursor advances
        /// to the last message in the batch.
        #[arg(long)]
        drain: bool,
        /// Persist the highest processed seq to this file and read it back as the
        /// floor on the next run — so you run the SAME command each turn and the
        /// read cursor only ever advances to messages you actually received.
        /// Takes precedence over --since-seq (which then just seeds the first
        /// run, before the file exists). This is the fix for "advanced my cursor
        /// past an unread peer message": track last-processed, never last-sent.
        /// Use one path unique to the server, room, and logical agent.
        #[arg(long)]
        cursor_file: Option<PathBuf>,
    },

    /// Monitor events in real-time
    Monitor {
        /// Filter to a specific room
        #[arg(long)]
        room: Option<String>,
        /// Output raw JSON frames
        #[arg(long)]
        json: bool,
    },

    /// Interactive persistent session for room coordination
    Shell {
        /// Optional room ID or name to join on start
        #[arg(long)]
        room: Option<String>,
    },

    /// Show server status
    Status,

    /// Generate a random end-to-end room key (for COWCHAT_ROOM_KEY). Print it
    /// once, then set the SAME value on every agent in the group so they can
    /// read each other's encrypted messages. No server connection needed.
    Keygen,

    /// Print the agent skill embedded in this binary (behavioral rules for
    /// coordinating over Cowchat), so the printed doctrine always matches the
    /// installed version. No server connection needed.
    Skill {
        /// Print the full command & protocol reference (SKILLS.md) instead
        #[arg(long)]
        full: bool,
    },

    /// Webhook subscriptions — register an HTTP endpoint to be POSTed when
    /// matching messages land in a room. Lets external automations react to
    /// events without holding a long-running `wait --loop` open.
    Sub {
        #[command(subcommand)]
        action: SubAction,
    },

    /// Room invites — mint a token a stranger redeems over HTTPS for a fresh
    /// API key plus access to one room. Replaces sharing a raw API key.
    Invites {
        #[command(subcommand)]
        action: InviteAction,
    },

    /// Export a room's history as markdown (or json/text).
    Export {
        /// Room ID or exact name
        room: String,
        /// Output format
        #[arg(long, default_value = "md")]
        format: ExportFormat,
        /// Only include messages with seq > this value
        #[arg(long)]
        since_seq: Option<i64>,
        /// Maximum messages to include (default: all)
        #[arg(long)]
        limit: Option<u32>,
        /// Include `thinking` pulses in the export. Off by default — most
        /// archives want only the chat narrative.
        #[arg(long)]
        include_thinking: bool,
        /// Write to file instead of stdout
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
    },

    /// Voting commands
    Vote {
        #[command(subcommand)]
        action: VoteAction,
    },

    /// Leader election commands
    Election {
        #[command(subcommand)]
        action: ElectionAction,
    },

    /// Set agent presence status (idle, waiting, working, thinking)
    Presence {
        /// Status: idle, waiting, working, or thinking
        status: String,
        /// Human-readable detail, e.g. "reviewing section 3"
        #[arg(long)]
        detail: Option<String>,
        /// Progress percentage (0-100)
        #[arg(long)]
        progress: Option<u8>,
    },

    /// LANTERN: an optional structured-reasoning overlay (HELLO + falsifiable
    /// claims, challenges, resolutions, synthesis). Carried inside message
    /// content — no server changes; state is reconstructed from history. Use it
    /// when a conversation is contested, high-stakes, or state-changing.
    Lantern {
        #[command(subcommand)]
        action: LanternAction,
    },
}

#[derive(Subcommand)]
enum LanternAction {
    /// Send a HELLO provenance preamble (identity, role, capability claims —
    /// all self-attested; advertises, does NOT grant, permissions).
    Hello {
        /// Room ID or name
        room: String,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        role: Option<String>,
        /// Repeatable capability as `name` or `name=falsifiable_by`.
        #[arg(long = "capability")]
        capabilities: Vec<String>,
    },
    /// Open a thread with a question.
    Probe {
        room: String,
        question: String,
        #[arg(long)]
        intent: Option<String>,
    },
    /// Make a falsifiable claim (opens a thread). `--falsifiable-by` is required.
    Assert {
        room: String,
        #[arg(long)]
        claim: String,
        #[arg(long)]
        confidence: Option<f64>,
        #[arg(long = "falsifiable-by")]
        falsifiable_by: String,
        #[arg(long)]
        intent: Option<String>,
    },
    /// Counter an ASSERT/CHALLENGE in a thread. Must stake confidence + a test.
    Challenge {
        room: String,
        #[arg(long)]
        thread: i64,
        #[arg(long = "target-seq")]
        target_seq: i64,
        #[arg(long = "counter-claim")]
        counter_claim: String,
        #[arg(long)]
        confidence: f64,
        #[arg(long)]
        test: String,
    },
    /// Record the observation that settles a branch (basis: tool/artifact/human/consensus/stale).
    Resolve {
        room: String,
        #[arg(long)]
        thread: i64,
        #[arg(long)]
        observation: String,
        #[arg(long)]
        basis: String,
    },
    /// Commit synthesis + shared-state delta into a thread (the commit point).
    Fuse {
        room: String,
        #[arg(long)]
        thread: i64,
        #[arg(long)]
        synthesis: String,
        /// Path to a JSON file holding the shared_state_delta object.
        #[arg(long = "state-delta")]
        state_delta: Option<PathBuf>,
        /// Preserve an intentional split rather than committing one model.
        #[arg(long)]
        split: bool,
        /// Repeatable calibration verdict `<seq>=<true|false>`: did that staked
        /// claim hold? Only scored when the thread's RESOLVE basis is tool/artifact/human.
        #[arg(long = "outcome")]
        outcomes: Vec<String>,
    },
    /// Reconcile shared state without prose (a state hash + JSON diff file).
    Sync {
        room: String,
        #[arg(long)]
        thread: Option<i64>,
        #[arg(long = "state-hash")]
        state_hash: Option<String>,
        #[arg(long = "diff")]
        diff: Option<PathBuf>,
    },
    /// Introduce a scarce, orthogonal idea (side channel; answer with harvest/bury).
    Spark {
        room: String,
        #[arg(long)]
        seed: String,
        #[arg(long = "why-now")]
        why_now: String,
        #[arg(long = "smallest-test")]
        smallest_test: String,
    },
    /// Accept a SPARK into the working set.
    Harvest {
        room: String,
        #[arg(long = "spark-seq")]
        spark_seq: i64,
        #[arg(long)]
        becomes: Option<String>,
    },
    /// Decline a SPARK in one sentence.
    Bury {
        room: String,
        #[arg(long = "spark-seq")]
        spark_seq: i64,
        #[arg(long)]
        reason: String,
    },
    /// List threads in a room (id, state, headline).
    Threads { room: String },
    /// Show every message in one thread, in order.
    Show { room: String, thread: i64 },
    /// Show the committed shared-state deltas (the agreed model).
    State { room: String },
    /// Show per-agent calibration loss (lower is better; diagnostic only).
    Calibration { room: String },
    /// Validate a LANTERN envelope from a file (or `-` for stdin).
    Validate { path: String },
}

#[derive(Subcommand)]
enum RoomAction {
    /// List all rooms
    List {
        /// Filter by parent room ID
        #[arg(long)]
        parent: Option<String>,
    },
    /// Create a new room
    Create {
        /// Room name
        name: String,
        /// Room description
        #[arg(long)]
        description: Option<String>,
        /// Parent room ID
        #[arg(long)]
        parent: Option<String>,
        /// Create as public: visible and joinable by any API key on the server.
        /// Default is private (only your API-key or keyless-local boundary can
        /// resolve it by name) — use this so other principals can find the room.
        #[arg(long)]
        public: bool,
        /// Create as end-to-end encrypted. Members must share a room key
        /// (--room-key or $COWCHAT_ROOM_KEY); the server stores only
        /// ciphertext and rejects plaintext sends to this room.
        #[arg(long)]
        encrypted: bool,
    },
    /// Get room info
    Info {
        /// Room ID
        room_id: String,
    },
    /// Get the latest seq for a room (the room's "tip")
    Tip {
        /// Room ID or exact name
        room: String,
    },
    /// Rename a room using its owning principal and recorded creator ID.
    Rename {
        /// Room ID or exact name
        room: String,
        /// New room name
        name: String,
    },
    /// Irreversibly remove a room from Cowchat using its owning principal and creator ID.
    Destroy {
        /// Room ID or exact name
        room: String,
        /// Confirm the irreversible deletion.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum VoteAction {
    /// Create a sealed-ballot vote in a room
    Create {
        /// Room ID
        room: String,
        /// Vote title / question
        title: String,
        /// Vote options (at least 2)
        #[arg(long, num_args = 2.., required = true)]
        options: Vec<String>,
        /// Optional description
        #[arg(long)]
        description: Option<String>,
        /// Deadline in seconds
        #[arg(long)]
        duration: Option<u64>,
    },
    /// Cast a ballot on an active vote
    Cast {
        /// Vote ID
        vote_id: String,
        /// Option index (0-based)
        option: usize,
    },
    /// Check status of a vote
    Status {
        /// Vote ID
        vote_id: String,
    },
    /// List recent votes in a room
    History {
        /// Room ID or exact room name
        room: String,
        /// Maximum number of votes to return
        #[arg(long, default_value = "20")]
        limit: u32,
    },
}

#[derive(Subcommand)]
enum ElectionAction {
    /// Start a leader election in a room
    Start {
        /// Room ID
        room: String,
    },
    /// Decline an active election
    Decline {
        /// Room ID
        room: String,
    },
    /// Issue a decision as room leader
    Decide {
        /// Room ID
        room: String,
        /// Decision content
        content: String,
    },
}

#[derive(Subcommand)]
enum SubAction {
    /// Create a new webhook subscription on a room.
    Create {
        /// Room ID or exact name
        room: String,
        /// Webhook URL (http or https). The server will POST messages here.
        #[arg(long)]
        url: String,
        /// HMAC-SHA256 shared secret used to sign each delivery (Standard
        /// Webhooks v1 signature). Keep it private — receivers verify with it.
        #[arg(long)]
        secret: String,
        /// Restrict to specific `--kind` values on the message (comma-separated).
        #[arg(long, value_delimiter = ',')]
        kinds: Vec<String>,
        /// Only deliver messages from this `--name`.
        #[arg(long)]
        only_from: Option<String>,
        /// Skip messages from this `--name`.
        #[arg(long)]
        not_from: Option<String>,
        /// Don't deliver `thinking` pulses (only real chat).
        #[arg(long)]
        exclude_thinking: bool,
        /// Start cursor. Integer, or `tip`/`auto` (default) for "only future
        /// messages", or `0` to backfill the entire room.
        #[arg(long, default_value = "tip")]
        since_seq: String,
    },
    /// List subscriptions owned by your API key.
    List {
        /// Optionally filter to one room.
        #[arg(long)]
        room: Option<String>,
    },
    /// Delete a subscription.
    Delete { subscription_id: String },
    /// Re-enable a `failed` subscription. Replays the backlog past the cursor.
    Enable { subscription_id: String },
}

#[derive(Subcommand)]
enum InviteAction {
    /// Mint an invite token for a room. Printed once — it cannot be recovered.
    Create {
        /// Room ID or exact name
        room: String,
        /// Make the invite open: redeemable repeatedly until revoked.
        /// Default is single-use (self-destructs on first redemption).
        #[arg(long)]
        open: bool,
    },
    /// List a room's invites (metadata only — tokens cannot be recovered).
    List {
        /// Room ID or exact name
        room: String,
    },
    /// Revoke an invite so no further redemption succeeds (creator or room
    /// owner only). Keys already minted through it keep their access.
    Revoke {
        /// The raw invite token (cinv_…) or the 64-hex invite id from
        /// `invites list`
        token_or_id: String,
    },
}

/// Classify a `invites revoke` argument: `cinv_…` values are raw tokens,
/// 64-hex values are invite ids (stored token hashes).
fn invite_revoke_is_id(value: &str) -> Result<bool, String> {
    if value.starts_with("cinv_") {
        Ok(false)
    } else if value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(true)
    } else {
        Err(format!(
            "'{value}' is neither an invite token (cinv_…) nor a 64-hex invite id"
        ))
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ExportFormat {
    /// Markdown — chat-style rendering with timestamps + agent labels.
    Md,
    /// One JSON object per message, newline-delimited.
    Json,
    /// Plain text — human-readable, no markup.
    Txt,
}

fn default_data_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".cowchat"))
        .unwrap_or_else(|| PathBuf::from(".cowchat"))
}

fn default_socket_path() -> &'static str {
    Box::leak(
        default_data_dir()
            .join("cowchat.sock")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str(),
    )
}

fn default_key_path() -> PathBuf {
    default_data_dir().join("auth.key")
}

fn load_key(key_arg: &Option<String>) -> String {
    if let Some(key) = key_arg {
        return key.clone();
    }
    let key_path = default_key_path();
    std::fs::read_to_string(key_path)
        .map(|key| key.trim().to_string())
        .unwrap_or_default()
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn resolve_agent_id(cli: &Cli) -> Option<String> {
    cli.agent_id
        .clone()
        .filter(|value| !value.is_empty())
        .or_else(|| env_non_empty("COWCHAT_AGENT_ID"))
}

fn command_represents_agent_session(command: &Commands) -> bool {
    matches!(
        command,
        Commands::Send { .. }
            | Commands::Thinking { .. }
            | Commands::Wait { .. }
            | Commands::Shell { .. }
            | Commands::Presence { .. }
    )
}

fn require_stable_named_agent(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    if cli.name != "cli"
        && command_represents_agent_session(&cli.command)
        && resolve_agent_id(cli).is_none()
    {
        return Err(format!(
            "named agent '{}' requires a stable identity; pass --agent-id <UNIQUE_TASK_AGENT_ID> or set COWCHAT_AGENT_ID",
            cli.name
        )
        .into());
    }
    Ok(())
}

pub(crate) fn resolve_room_key(flag: Option<String>) -> Option<String> {
    flag.filter(|v| !v.is_empty())
        .or_else(|| env_non_empty("COWCHAT_ROOM_KEY"))
}

/// Resolve the end-to-end room secret. Returns None when no flag or supported
/// environment variable is set (the client then sends/receives plaintext).
fn resolve_room_secret(cli: &Cli) -> Option<Vec<u8>> {
    resolve_room_key(cli.room_key.clone()).map(String::into_bytes)
}

/// Decrypt a raw frame `content` field for display. Used on pushed events
/// (monitor/shell/follow) where the client API hasn't already decrypted. Falls
/// back to the original string when there's no key or it isn't a ciphertext blob.
fn decrypt_field(secret: Option<&[u8]>, room_id: &str, content: &str) -> String {
    match secret {
        Some(s) if cowchat_core::crypto::is_ciphertext(content) => {
            cowchat_core::crypto::decrypt(s, room_id, content)
                .unwrap_or_else(|_| content.to_string())
        }
        _ => content.to_string(),
    }
}

async fn connect(cli: &Cli) -> Result<CowchatClient, Box<dyn std::error::Error>> {
    let key = load_key(&cli.key);

    let agent_id = resolve_agent_id(cli);
    let agent_id = agent_id.as_deref();
    let mut client = if let Some(url) = &cli.url {
        CowchatClient::connect_ws(url, &key, &cli.name, agent_id, vec![]).await?
    } else if let Some(addr) = &cli.tcp {
        CowchatClient::connect_tcp(addr, &key, &cli.name, agent_id, vec![]).await?
    } else {
        CowchatClient::connect_uds(&cli.socket, &key, &cli.name, agent_id, vec![]).await?
    };
    if let Some(secret) = resolve_room_secret(cli) {
        client.set_room_secret(&secret);
    }
    Ok(client)
}

async fn resolve_room_id(
    client: &CowchatClient,
    room: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    // Fast path: already a room ID.
    match client.room_info(room).await {
        Ok(_) => return Ok(room.to_string()),
        Err(ClientError::Server {
            code: ErrorCode::RoomNotFound,
            ..
        }) => {}
        Err(e) => return Err(Box::new(e)),
    }

    // Fallback: resolve by exact room name.
    let rooms = client.list_rooms(None).await?;
    let matches: Vec<_> = rooms.into_iter().filter(|r| r.name == room).collect();

    match matches.as_slice() {
        [single] => Ok(single.room_id.clone()),
        [] => Err(format!("Room '{room}' not found (expected ID or exact name)").into()),
        _ => Err(format!("Room name '{room}' is ambiguous; use the room ID").into()),
    }
}

/// HTTP base for the invite-redeem hint. The CLI only knows the server's HTTP
/// origin when `--url` (a ws/wss endpoint) was used; otherwise print a
/// path-only placeholder the operator fills in.
fn invite_http_base(ws_url: Option<&str>) -> String {
    match ws_url {
        Some(url) => {
            let base = if let Some(rest) = url.strip_prefix("wss://") {
                format!("https://{rest}")
            } else if let Some(rest) = url.strip_prefix("ws://") {
                format!("http://{rest}")
            } else {
                url.to_string()
            };
            base.trim_end_matches('/')
                .trim_end_matches("/ws")
                .trim_end_matches('/')
                .to_string()
        }
        None => "https://<host>".to_string(),
    }
}

fn write_cursor_atomic(path: &std::path::Path, seq: i64) -> std::io::Result<()> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("cursor");
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    std::fs::write(&temporary, seq.to_string())?;
    std::fs::rename(temporary, path)
}

fn advance_cursor(
    cursor: &mut Option<i64>,
    cursor_file: Option<&PathBuf>,
    seq: i64,
) -> Result<(), ClientError> {
    if let Some(path) = cursor_file {
        write_cursor_atomic(path, seq).map_err(ClientError::Io)?;
    }
    *cursor = Some(seq);
    Ok(())
}

fn is_retryable_wait_error(error: &ClientError) -> bool {
    matches!(
        error,
        ClientError::Io(_)
            | ClientError::ConnectionClosed
            | ClientError::Timeout
            | ClientError::Channel
            | ClientError::Ws(_)
    )
}

fn spawn_wait_presence_watcher(
    client: &CowchatClient,
    room_id: &str,
    enabled: bool,
) -> Option<tokio::task::JoinHandle<()>> {
    if !enabled {
        return None;
    }
    let mut events = client.subscribe();
    let room_label = room_id.to_string();
    Some(tokio::spawn(async move {
        while let Ok(evt) = events.recv().await {
            let in_room = evt.frame.payload.get("room_id").and_then(|v| v.as_str())
                == Some(room_label.as_str());
            if !in_room {
                continue;
            }
            match evt.frame.frame_type {
                FrameType::AgentJoined => {
                    let name = evt
                        .frame
                        .payload
                        .get("agent")
                        .and_then(|a| a.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    eprintln!("wait: peer {} joined", name);
                }
                FrameType::AgentLeft => {
                    let who = evt
                        .frame
                        .payload
                        .get("agent_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    eprintln!("wait: peer {} left", who);
                }
                _ => {}
            }
        }
    }))
}

fn spawn_wait_thinking_watcher(
    client: &CowchatClient,
    room_id: &str,
    self_name: &str,
    secret: Option<Vec<u8>>,
    enabled: bool,
) -> Option<tokio::task::JoinHandle<()>> {
    if !enabled {
        return None;
    }
    let mut events = client.subscribe();
    let room_label = room_id.to_string();
    let self_name = self_name.to_string();
    Some(tokio::spawn(async move {
        while let Ok(evt) = events.recv().await {
            if evt.frame.frame_type != FrameType::Thinking {
                continue;
            }
            let p = &evt.frame.payload;
            if p.get("room_id").and_then(|v| v.as_str()) != Some(room_label.as_str()) {
                continue;
            }
            let name = p.get("agent_name").and_then(|v| v.as_str()).unwrap_or("?");
            if name == self_name {
                continue;
            }
            let content = p.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let content = decrypt_field(secret.as_deref(), &room_label, content);
            eprintln!("wait: thinking {}: {}", name, content);
        }
    }))
}

#[allow(clippy::too_many_arguments)]
async fn run_wait_follow(
    cli: &Cli,
    room: &str,
    since_seq: Option<&str>,
    heartbeat_secs: u64,
    only_from: Option<&String>,
    not_from: Option<&String>,
    only_kind: Option<&String>,
    show_thinking: bool,
    text: bool,
    output: Option<&PathBuf>,
    cursor_file: Option<&PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cursor = cursor_file
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<i64>().ok());
    let mut backoff = 1u64;
    let started = std::time::Instant::now();
    let mut last_heartbeat = std::time::Instant::now();

    loop {
        let client = match connect(cli).await {
            Ok(client) => client,
            Err(error) => {
                eprintln!("wait --follow: connect failed: {error}; retrying in {backoff}s");
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(30);
                continue;
            }
        };
        let room_id = match resolve_room_id(&client, room).await {
            Ok(id) => id,
            Err(error) => {
                eprintln!("wait --follow: room lookup failed: {error}; retrying in {backoff}s");
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(30);
                continue;
            }
        };
        if cursor.is_none() {
            cursor = match since_seq {
                None => match client.room_tip(&room_id).await {
                    Ok(tip) => Some(tip),
                    Err(error) => {
                        eprintln!(
                            "wait --follow: tip lookup failed: {error}; retrying in {backoff}s"
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                        backoff = (backoff * 2).min(30);
                        continue;
                    }
                },
                Some(s) if s.eq_ignore_ascii_case("tip") || s.eq_ignore_ascii_case("auto") => {
                    match client.room_tip(&room_id).await {
                        Ok(tip) => Some(tip),
                        Err(error) => {
                            eprintln!(
                                "wait --follow: tip lookup failed: {error}; retrying in {backoff}s"
                            );
                            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                            backoff = (backoff * 2).min(30);
                            continue;
                        }
                    }
                }
                Some(s) => Some(s.parse::<i64>().map_err(|e| {
                    format!("--since-seq must be an integer, 'tip', or 'auto': {e}")
                })?),
            };
        }
        if let Err(error) = client.join_room(&room_id).await {
            eprintln!("wait --follow: join failed: {error}; retrying in {backoff}s");
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(30);
            continue;
        }
        let _ = client.set_presence("waiting", None, None).await;
        backoff = 1;

        let connection_result: Result<(), ClientError> = async {
            loop {
                // Pulling history before waiting makes the cursor authoritative
                // across disconnects and broadcast lag. Advance over filtered,
                // self, thinking, and system rows too, so none can pin recovery.
                let mut batch = client
                    .get_history_filtered(&room_id, 500, None, None, cursor)
                    .await?;
                batch.sort_by_key(|message| message.seq);
                if batch.is_empty() {
                    if let Some(message) = client.wait_for_message(&room_id, 5, cursor).await? {
                        batch.push(message);
                    }
                }

                for message in batch {
                    if cursor.is_some_and(|seq| message.seq <= seq) {
                        continue;
                    }
                    let row_type = message.metadata.get("type").and_then(|v| v.as_str());
                    if row_type == Some("thinking") {
                        if show_thinking && !client.is_self_message(&message) {
                            eprintln!("wait: thinking {}: {}", message.agent_name, message.content);
                        }
                        advance_cursor(&mut cursor, cursor_file, message.seq)?;
                        continue;
                    }
                    if row_type == Some("system") || client.is_self_message(&message) {
                        advance_cursor(&mut cursor, cursor_file, message.seq)?;
                        continue;
                    }
                    if only_from.is_some_and(|name| message.agent_name != *name)
                        || not_from.is_some_and(|name| message.agent_name == *name)
                        || only_kind.is_some_and(|kind| {
                            message.metadata.get("kind").and_then(|v| v.as_str())
                                != Some(kind.as_str())
                        })
                    {
                        advance_cursor(&mut cursor, cursor_file, message.seq)?;
                        continue;
                    }

                    let rendered = if text {
                        format_message(&message)
                    } else {
                        serde_json::to_string(&message).unwrap_or_default()
                    };
                    if let Some(path) = output {
                        let mut file = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(path)
                            .map_err(ClientError::Io)?;
                        writeln!(file, "{rendered}").map_err(ClientError::Io)?;
                    } else {
                        println!("{rendered}");
                    }
                    advance_cursor(&mut cursor, cursor_file, message.seq)?;
                    if message.metadata.get("kind").and_then(|v| v.as_str())
                        == Some(KIND_CONVERSATION_END)
                    {
                        eprintln!("Peer ended the conversation.");
                        std::process::exit(3);
                    }
                }

                if heartbeat_secs > 0
                    && last_heartbeat.elapsed() >= std::time::Duration::from_secs(heartbeat_secs)
                {
                    eprintln!(
                        "wait: alive {}s room={} since_seq={} mode=follow",
                        started.elapsed().as_secs(),
                        room_id,
                        cursor.unwrap_or(0)
                    );
                    last_heartbeat = std::time::Instant::now();
                }
            }
        }
        .await;
        let _ = client.set_presence("idle", None, None).await;
        if let Err(error) = connection_result {
            eprintln!("wait --follow: connection lost: {error}; retrying in {backoff}s");
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(30);
        }
    }
}

fn print_shell_help() {
    println!("Interactive shell commands:");
    println!("  /help                 Show this help");
    println!("  /join <room>          Join room (id or exact name) and make it active");
    println!("  /leave [room]         Leave active room (or explicit room)");
    println!("  /room                 Show current active room");
    println!("  /rooms                List rooms");
    println!("  /agents               List agents in active room");
    println!("  /history [limit]      Show room history (default 20)");
    println!("  /send <message>       Send message to active room");
    println!("  /quit                 Exit shell");
    println!("  <text>                Shortcut for /send <text>");
}

fn print_shell_prompt(current_room: Option<&str>) {
    let room = current_room.unwrap_or("no-room");
    print!("cowchat[{room}]> ");
    let _ = std::io::stdout().flush();
}

async fn run_shell(
    cli: &Cli,
    start_room: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = connect(cli).await?;
    let room_secret = resolve_room_secret(cli);
    let mut current_room: Option<String> = None;

    if let Some(room_ref) = start_room {
        let room_id = resolve_room_id(&client, room_ref).await?;
        client.join_room(&room_id).await?;
        println!("Joined room: {}", room_id);
        current_room = Some(room_id);
    }

    println!(
        "Connected as '{}' (agent_id: {})",
        client.agent_name, client.agent_id
    );
    print_shell_help();

    let mut stdin_lines = BufReader::new(tokio::io::stdin()).lines();
    let mut events = client.subscribe();

    loop {
        print_shell_prompt(current_room.as_deref());

        tokio::select! {
            line = stdin_lines.next_line() => {
                let Some(line) = line? else {
                    println!();
                    break;
                };

                let input = line.trim();
                if input.is_empty() {
                    continue;
                }

                if let Some(command_text) = input.strip_prefix('/') {
                    let (cmd, rest) = match command_text.split_once(' ') {
                        Some((cmd, rest)) => (cmd.trim(), rest.trim()),
                        None => (command_text.trim(), ""),
                    };

                    match cmd {
                        "help" => print_shell_help(),
                        "join" => {
                            if rest.is_empty() {
                                println!("Usage: /join <room-id-or-name>");
                                continue;
                            }
                            match resolve_room_id(&client, rest).await {
                                Ok(room_id) => {
                                    match client.join_room(&room_id).await {
                                        Ok(_) => {
                                            println!("Joined room: {}", room_id);
                                            current_room = Some(room_id);
                                        }
                                        Err(e) => println!("Join failed: {}", e),
                                    }
                                }
                                Err(e) => println!("Join failed: {}", e),
                            }
                        }
                        "leave" => {
                            let target_room = if rest.is_empty() {
                                current_room.clone()
                            } else {
                                match resolve_room_id(&client, rest).await {
                                    Ok(id) => Some(id),
                                    Err(e) => {
                                        println!("Leave failed: {}", e);
                                        None
                                    }
                                }
                            };

                            if let Some(room_id) = target_room {
                                match client.leave_room(&room_id).await {
                                    Ok(_) => {
                                        println!("Left room: {}", room_id);
                                        if current_room.as_deref() == Some(room_id.as_str()) {
                                            current_room = None;
                                        }
                                    }
                                    Err(e) => println!("Leave failed: {}", e),
                                }
                            } else {
                                println!("No active room to leave.");
                            }
                        }
                        "room" => {
                            match current_room.as_deref() {
                                Some(room_id) => println!("Active room: {}", room_id),
                                None => println!("No active room. Use /join <room> first."),
                            }
                        }
                        "rooms" => {
                            match client.list_rooms(None).await {
                                Ok(rooms) => {
                                    if rooms.is_empty() {
                                        println!("No rooms found.");
                                    } else {
                                        println!("{:<38} {:<20} DESCRIPTION", "ID", "NAME");
                                        println!("{}", "-".repeat(80));
                                        for room in rooms {
                                            let desc = room.description.as_deref().unwrap_or("");
                                            println!("{:<38} {:<20} {}", room.room_id, room.name, desc);
                                        }
                                    }
                                }
                                Err(e) => println!("Failed to list rooms: {}", e),
                            }
                        }
                        "agents" => {
                            match client.list_agents(current_room.as_deref()).await {
                                Ok(agents) => {
                                    if agents.is_empty() {
                                        println!("No agents connected.");
                                    } else {
                                        println!(
                                            "{:<38} {:<16} {:<10} {:<10} DETAIL",
                                            "AGENT ID", "NAME", "STATUS", "PROGRESS"
                                        );
                                        println!("{}", "-".repeat(100));
                                        for agent in agents {
                                            let status = agent.status.as_deref().unwrap_or("-");
                                            let progress = agent
                                                .progress
                                                .map(|p| format!("{}%", p))
                                                .unwrap_or_default();
                                            let detail = agent.status_detail.as_deref().unwrap_or("");
                                            println!(
                                                "{:<38} {:<16} {:<10} {:<10} {}",
                                                agent.agent_id, agent.name, status, progress, detail
                                            );
                                        }
                                    }
                                }
                                Err(e) => println!("Failed to list agents: {}", e),
                            }
                        }
                        "history" => {
                            let limit = if rest.is_empty() {
                                20
                            } else {
                                match rest.parse::<u32>() {
                                    Ok(v) => v,
                                    Err(_) => {
                                        println!("Usage: /history [limit]");
                                        continue;
                                    }
                                }
                            };

                            let Some(room_id) = current_room.as_deref() else {
                                println!("No active room. Use /join <room> first.");
                                continue;
                            };

                            match client.get_history(room_id, limit, None).await {
                                Ok(messages) => {
                                    for msg in messages {
                                        println!("{}", format_message(&msg));
                                    }
                                }
                                Err(e) => println!("Failed to load history: {}", e),
                            }
                        }
                        "send" => {
                            if rest.is_empty() {
                                println!("Usage: /send <message>");
                                continue;
                            }
                            if let Some(room_id) = current_room.as_deref() {
                                if let Err(e) = client.send_message(room_id, rest, None, vec![]).await {
                                    println!("Send failed: {}", e);
                                }
                            } else {
                                println!("No active room. Use /join <room> first.");
                            }
                        }
                        "quit" | "exit" => break,
                        _ => {
                            println!("Unknown command: /{} (try /help)", cmd);
                        }
                    }
                } else if let Some(room_id) = current_room.as_deref() {
                    if let Err(e) = client.send_message(room_id, input, None, vec![]).await {
                        println!("Send failed: {}", e);
                    }
                } else {
                    println!("No active room. Use /join <room> first.");
                }
            }
            event = events.recv() => {
                match event {
                    Ok(event) => {
                        if let Some(active_room) = current_room.as_deref() {
                            if let Some(event_room) = event.frame.payload.get("room_id").and_then(|v| v.as_str()) {
                                if event_room != active_room {
                                    continue;
                                }
                            }
                        }
                        println!();
                        print_event(&event.frame, room_secret.as_deref());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        println!("\n[warn] event stream lagged (dropped {} events)", skipped);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        println!("\n[event stream closed]");
                        break;
                    }
                }
            }
        }
    }

    println!("Goodbye.");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let cli = Cli::parse();
    require_stable_named_agent(&cli)?;

    match &cli.command {
        Commands::Send {
            room,
            message,
            reply_to,
            kind,
            end,
        } => {
            let client = connect(&cli).await?;
            let room_id = resolve_room_id(&client, room).await?;
            client.join_room(&room_id).await?;
            // `--end` is sugar for `--kind conversation_end` (the two conflict at
            // the arg level, so at most one is set).
            let kind = if *end {
                Some(KIND_CONVERSATION_END.to_string())
            } else {
                kind.clone()
            };
            let msg = match &kind {
                Some(k) => {
                    client
                        .send_message_with_metadata(
                            &room_id,
                            message,
                            reply_to.as_deref(),
                            vec![],
                            serde_json::json!({ "kind": k }),
                        )
                        .await?
                }
                None => {
                    client
                        .send_message(&room_id, message, reply_to.as_deref(), vec![])
                        .await?
                }
            };
            println!("{}", format_message(&msg));
        }

        Commands::Thinking { room, content } => {
            let client = connect(&cli).await?;
            let room_id = resolve_room_id(&client, room).await?;
            client.join_room(&room_id).await?;
            let msg = client.thinking(&room_id, content).await?;
            println!("{}", format_message(&msg));
        }

        Commands::Rooms { action } => {
            let client = connect(&cli).await?;
            match action {
                RoomAction::List { parent } => {
                    let rooms = client.list_rooms(parent.as_deref()).await?;
                    if rooms.is_empty() {
                        println!("No rooms found.");
                    } else {
                        println!(
                            "{:<38} {:<20} {:<8} {:<20} DESCRIPTION",
                            "ID", "NAME", "MEMBERS", "LAST ACTIVITY"
                        );
                        println!("{}", "-".repeat(120));
                        for room in rooms {
                            let desc = room.description.as_deref().unwrap_or("");
                            let members = room
                                .member_count
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| "-".to_string());
                            let activity = room
                                .last_activity
                                .map(|t| t.format("%H:%M:%S").to_string())
                                .unwrap_or_else(|| "-".to_string());
                            println!(
                                "{:<38} {:<20} {:<8} {:<20} {}",
                                room.room_id, room.name, members, activity, desc
                            );
                        }
                    }
                }
                RoomAction::Create {
                    name,
                    description,
                    parent,
                    public,
                    encrypted,
                } => {
                    let room = client
                        .create_room_with_options(
                            name,
                            description.as_deref(),
                            parent.as_deref(),
                            *public,
                            *encrypted,
                        )
                        .await?;
                    println!("Created room: {} ({})", room.name, room.room_id);
                    println!(
                        "  Visibility: {}",
                        if room.visibility == "public" {
                            "public (any key can find & join)"
                        } else {
                            "private (owning key or keyless-local boundary)"
                        }
                    );
                    if room.encrypted {
                        println!("  End-to-end encrypted: members need a shared room key");
                        if resolve_room_secret(&cli).is_none() {
                            println!(
                                "  Note: no room key set — pass --room-key or set $COWCHAT_ROOM_KEY to send/read here"
                            );
                        }
                    }
                }
                RoomAction::Info { room_id } => {
                    let resolved = resolve_room_id(&client, room_id).await?;
                    let info = client.room_info(&resolved).await?;
                    println!("{}", serde_json::to_string_pretty(&info)?);
                }
                RoomAction::Tip { room } => {
                    let room_id = resolve_room_id(&client, room).await?;
                    let seq = client.room_tip(&room_id).await?;
                    println!("{}", seq);
                }
                RoomAction::Rename { room, name } => {
                    let room_id = resolve_room_id(&client, room).await?;
                    let updated = client.rename_room(&room_id, name).await?;
                    println!("Renamed room: {} ({})", updated.name, updated.room_id);
                }
                RoomAction::Destroy { room, yes } => {
                    if !yes {
                        return Err(
                            "room destruction is irreversible; re-run with --yes to confirm".into(),
                        );
                    }
                    let room_id = resolve_room_id(&client, room).await?;
                    client.destroy_room(&room_id).await?;
                    println!("Destroyed room: {}", room_id);
                }
            }
        }

        Commands::Agents { room } => {
            let client = connect(&cli).await?;
            let agents = client.list_agents(room.as_deref()).await?;

            // If --room is set, also pull recent history so we can surface
            // agents who've been posting recently even if they're not currently
            // connected (each CLI invocation registers + disconnects, so an
            // active reviewer flickers in and out of the live agents list).
            let (last_in_room, room_id_for_history): (
                std::collections::HashMap<String, (i64, String)>,
                Option<String>,
            ) = if let Some(r) = room {
                let room_id = resolve_room_id(&client, r).await?;
                let hist = client
                    .get_history(&room_id, 200, None)
                    .await
                    .unwrap_or_default();
                let mut map = std::collections::HashMap::new();
                for m in hist.iter().rev() {
                    // Iterate newest-first; keep first sighting per agent_name.
                    map.entry(m.agent_name.clone())
                        .or_insert((m.seq, m.timestamp.format("%H:%M:%S").to_string()));
                }
                (map, Some(room_id))
            } else {
                (std::collections::HashMap::new(), None)
            };

            let show_room_activity = room_id_for_history.is_some();
            let live_names: std::collections::HashSet<String> =
                agents.iter().map(|a| a.name.clone()).collect();

            if agents.is_empty() && last_in_room.is_empty() {
                println!("No agents connected; no recent activity in room.");
            } else {
                if show_room_activity {
                    println!(
                        "{:<10} {:<38} {:<16} {:<10} {:<8} {:<10} {:<10} DETAIL",
                        "STATE", "AGENT ID", "NAME", "STATUS", "PROG", "ACTIVE", "LAST_SEQ"
                    );
                    println!("{}", "-".repeat(130));
                } else {
                    println!(
                        "{:<38} {:<16} {:<10} {:<8} {:<10} DETAIL",
                        "AGENT ID", "NAME", "STATUS", "PROG", "ACTIVE"
                    );
                    println!("{}", "-".repeat(110));
                }

                // First: currently connected agents.
                for agent in &agents {
                    let status = agent.status.as_deref().unwrap_or("-");
                    let progress = agent
                        .progress
                        .map(|p| format!("{}%", p))
                        .unwrap_or_default();
                    let active = agent
                        .last_active
                        .map(|t| t.format("%H:%M:%S").to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let detail = agent.status_detail.as_deref().unwrap_or("");
                    if show_room_activity {
                        let (last_seq_s, _last_ts_s) = last_in_room
                            .get(&agent.name)
                            .cloned()
                            .unwrap_or((0, "-".to_string()));
                        let last_seq = if last_seq_s > 0 {
                            format!("#{}", last_seq_s)
                        } else {
                            "-".to_string()
                        };
                        println!(
                            "{:<10} {:<38} {:<16} {:<10} {:<8} {:<10} {:<10} {}",
                            "LIVE",
                            agent.agent_id,
                            agent.name,
                            status,
                            progress,
                            active,
                            last_seq,
                            detail
                        );
                    } else {
                        println!(
                            "{:<38} {:<16} {:<10} {:<8} {:<10} {}",
                            agent.agent_id, agent.name, status, progress, active, detail
                        );
                    }
                }

                // Then: agents seen in recent history but NOT currently connected.
                // These are the ghosts the user actually cares about: "has codex
                // been here recently even though they just disconnected?"
                if show_room_activity {
                    for (name, (seq, ts)) in &last_in_room {
                        if live_names.contains(name) {
                            continue;
                        }
                        println!(
                            "{:<10} {:<38} {:<16} {:<10} {:<8} {:<10} {:<10} (last seen via history)",
                            "RECENT",
                            "-",
                            name,
                            "-",
                            "",
                            ts,
                            format!("#{}", seq)
                        );
                    }
                }
            }
        }

        Commands::History {
            room,
            limit,
            follow,
            since,
            since_seq,
            kind,
            output,
        } => {
            let client = connect(&cli).await?;
            let room_id = resolve_room_id(&client, room).await?;

            // Show history (with optional --since / --since-seq filter)
            let messages = client
                .get_history_filtered(&room_id, *limit, None, since.as_deref(), *since_seq)
                .await?;

            // Optional kind filter (applied client-side).
            let filtered: Vec<&ChatMessage> = messages
                .iter()
                .filter(|m| match kind {
                    Some(k) => m.metadata.get("kind").and_then(|v| v.as_str()) == Some(k.as_str()),
                    None => true,
                })
                .collect();

            let body: String = filtered
                .iter()
                .map(|m| format_message(m))
                .collect::<Vec<_>>()
                .join("\n");

            match output {
                Some(path) => {
                    std::fs::write(path, format!("{}\n", body))?;
                    eprintln!("wrote {} entries to {}", filtered.len(), path.display());
                }
                None => {
                    if !body.is_empty() {
                        println!("{}", body);
                    }
                }
            }

            if *follow {
                // Join the room to receive new messages
                let _ = client.join_room(&room_id).await;
                let room_secret = resolve_room_secret(&cli);
                let mut events = client.subscribe();
                println!("--- streaming new messages (Ctrl+C to stop) ---");
                while let Ok(event) = events.recv().await {
                    if event.frame.frame_type == FrameType::MessageReceived {
                        if let Some(event_room_id) =
                            event.frame.payload.get("room_id").and_then(|v| v.as_str())
                        {
                            if event_room_id == room_id {
                                let agent_name = event
                                    .frame
                                    .payload
                                    .get("agent_name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("?");
                                let content = event
                                    .frame
                                    .payload
                                    .get("content")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let content =
                                    decrypt_field(room_secret.as_deref(), &room_id, content);
                                let ts = event
                                    .frame
                                    .payload
                                    .get("timestamp")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                println!(
                                    "[{}] {}: {}",
                                    &ts[11..19.min(ts.len())],
                                    agent_name,
                                    content
                                );
                            }
                        }
                    }
                }
            }
        }

        Commands::Wait {
            room,
            timeout,
            json: _json,
            text,
            since_seq,
            loop_,
            follow,
            idle_timeout,
            heartbeat_secs,
            only_from,
            not_from,
            only_kind,
            show_thinking,
            output,
            drain,
            cursor_file,
        } => {
            if *follow {
                run_wait_follow(
                    &cli,
                    room,
                    since_seq.as_deref(),
                    *heartbeat_secs,
                    only_from.as_ref(),
                    not_from.as_ref(),
                    only_kind.as_ref(),
                    *show_thinking,
                    *text,
                    output.as_ref(),
                    cursor_file.as_ref(),
                )
                .await?;
                return Ok(());
            }
            let mut client = connect(&cli).await?;
            let mut room_id = resolve_room_id(&client, room).await?;
            let _ = client.join_room(&room_id).await;

            // A cursor file, when present, is the source of truth for the read
            // floor: it holds the highest seq we've actually processed, so the
            // floor never jumps ahead to our own sent message. It overrides
            // --since-seq, which then only seeds the first run (file absent).
            let cursor_seq: Option<i64> = cursor_file.as_ref().and_then(|p| {
                std::fs::read_to_string(p)
                    .ok()
                    .and_then(|s| s.trim().parse::<i64>().ok())
            });

            // Resolve `--since-seq tip|auto` to the room's current tip. Done
            // BEFORE the wait subscribes so we don't miss anything arriving
            // between the tip read and the subscribe (the SDK's wait subscribes
            // first, then checks history with since_seq — that closes the race).
            let resolved_since_seq: Option<i64> = if let Some(seq) = cursor_seq {
                Some(seq)
            } else {
                match since_seq.as_deref() {
                    None => None,
                    Some(s) if s.eq_ignore_ascii_case("tip") || s.eq_ignore_ascii_case("auto") => {
                        Some(client.room_tip(&room_id).await?)
                    }
                    Some(s) => Some(s.parse::<i64>().map_err(|e| {
                        Box::<dyn std::error::Error>::from(format!(
                            "--since-seq must be an integer, 'tip', or 'auto': {}",
                            e
                        ))
                    })?),
                }
            };

            // Announce we're waiting so other agents in the room know someone is blocked.
            // Best-effort — don't fail the wait if presence broadcast fails.
            let _ = client.set_presence("waiting", None, None).await;

            let effective_timeout = if *timeout == 0 { 86400 } else { *timeout }; // 0 = 24h

            // Heartbeat task: periodically prints to stderr so tool wrappers that kill
            // silent processes see liveness. Aborted as soon as wait returns.
            let heartbeat_task = if *heartbeat_secs > 0 {
                let interval = *heartbeat_secs;
                let room_label = room_id.clone();
                let since_label = resolved_since_seq
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let mode_label = if *loop_ { "loop" } else { "once" };
                Some(tokio::spawn(async move {
                    let started = std::time::Instant::now();
                    let mut tick = tokio::time::interval(std::time::Duration::from_secs(interval));
                    tick.tick().await; // skip the immediate first tick
                    loop {
                        tick.tick().await;
                        eprintln!(
                            "wait: alive {}s room={} since_seq={} mode={}",
                            started.elapsed().as_secs(),
                            room_label,
                            since_label,
                            mode_label,
                        );
                    }
                }))
            } else {
                None
            };

            // Presence-watcher task: prints peer join/leave to stderr while the
            // wait blocks, so a waiting agent (or the human) can see the other
            // side arrive and know to keep waiting instead of concluding "gone".
            // Uses its own event subscription, independent of the SDK wait loop.
            // Gated by the same quiet switch as heartbeats.
            let mut presence_task =
                spawn_wait_presence_watcher(&client, &room_id, *heartbeat_secs > 0);

            // Thinking-watcher task: with --show-thinking, print peers' thinking
            // pulses to stderr for live visibility, WITHOUT waking the wait (it
            // still only returns on a real chat message). Own pulses are skipped;
            // content is decrypted if a room key is configured.
            let room_secret = resolve_room_secret(&cli);
            let mut thinking_task = spawn_wait_thinking_watcher(
                &client,
                &room_id,
                &cli.name,
                room_secret.clone(),
                *show_thinking,
            );

            // Helper closure: does a candidate message match all filters?
            let matches = |msg: &ChatMessage| -> bool {
                if let Some(want) = only_from {
                    if &msg.agent_name != want {
                        return false;
                    }
                }
                if let Some(skip) = not_from {
                    if &msg.agent_name == skip {
                        return false;
                    }
                }
                if let Some(want_kind) = only_kind {
                    let got_kind = msg.metadata.get("kind").and_then(|v| v.as_str());
                    if got_kind != Some(want_kind.as_str()) {
                        return false;
                    }
                }
                true
            };

            // Inner loop: with --loop, keep advancing the bookmark until the
            // returned message passes all filters. Without --loop, do at most
            // one underlying wait call. In --loop mode the loop never returns on
            // its own until a match arrives, so an optional idle deadline races it.
            // `latest_seq` mirrors the bookmark out of the future (which the
            // select! may drop) so the idle-expiry path can print an accurate
            // resume point even after filters advanced past some messages.
            // i64::MIN is the "never advanced" sentinel (resume from `tip`).
            let latest_seq =
                std::sync::atomic::AtomicI64::new(resolved_since_seq.unwrap_or(i64::MIN));
            let wait_loop = async {
                let mut cursor = resolved_since_seq;
                let mut backoff = 1u64;
                loop {
                    let result = client
                        .wait_for_message(&room_id, effective_timeout, cursor)
                        .await;
                    match result {
                        Ok(Some(msg)) => {
                            backoff = 1;
                            if matches(&msg) {
                                break Ok(Some(msg));
                            }
                            // Filter rejected — advance the bookmark and continue (only
                            // makes sense in --loop mode; without --loop fall through
                            // and return None so the caller sees a timeout-like signal).
                            if !*loop_ {
                                break Ok(None);
                            }
                            cursor = Some(msg.seq);
                            latest_seq.store(msg.seq, std::sync::atomic::Ordering::Relaxed);
                        }
                        Ok(None) if *loop_ => continue,
                        Ok(None) => break Ok(None),
                        Err(error) if *loop_ && is_retryable_wait_error(&error) => {
                            eprintln!(
                                "wait --loop: connection lost: {error}; retrying in {backoff}s"
                            );
                            if let Some(task) = presence_task.take() {
                                task.abort();
                            }
                            if let Some(task) = thinking_task.take() {
                                task.abort();
                            }

                            loop {
                                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                                let replacement = match connect(&cli).await {
                                    Ok(replacement) => replacement,
                                    Err(reconnect_error) => {
                                        eprintln!(
                                            "wait --loop: reconnect failed: {reconnect_error}; retrying in {}s",
                                            (backoff * 2).min(30)
                                        );
                                        backoff = (backoff * 2).min(30);
                                        continue;
                                    }
                                };
                                let replacement_room_id = match resolve_room_id(&replacement, room)
                                    .await
                                {
                                    Ok(id) => id,
                                    Err(reconnect_error) => {
                                        eprintln!(
                                            "wait --loop: room lookup failed: {reconnect_error}; retrying in {}s",
                                            (backoff * 2).min(30)
                                        );
                                        backoff = (backoff * 2).min(30);
                                        continue;
                                    }
                                };
                                if let Err(reconnect_error) =
                                    replacement.join_room(&replacement_room_id).await
                                {
                                    eprintln!(
                                        "wait --loop: join failed: {reconnect_error}; retrying in {}s",
                                        (backoff * 2).min(30)
                                    );
                                    backoff = (backoff * 2).min(30);
                                    continue;
                                }

                                client = replacement;
                                room_id = replacement_room_id;
                                let _ = client.set_presence("waiting", None, None).await;
                                presence_task = spawn_wait_presence_watcher(
                                    &client,
                                    &room_id,
                                    *heartbeat_secs > 0,
                                );
                                thinking_task = spawn_wait_thinking_watcher(
                                    &client,
                                    &room_id,
                                    &cli.name,
                                    room_secret.clone(),
                                    *show_thinking,
                                );
                                backoff = 1;
                                break;
                            }
                        }
                        Err(error) => break Err(error),
                    }
                }
            };

            // `idle_expired` is only reachable in --loop mode with a non-zero
            // deadline; otherwise we just await the loop.
            let mut idle_expired = false;
            let matched: Result<Option<ChatMessage>, _> = if *loop_ && *idle_timeout > 0 {
                // On expiry the loser future is dropped; a message that landed in
                // the broadcast buffer in that instant is discarded, but the resume
                // hint below (from --since-seq) lets the caller catch up on re-run.
                tokio::select! {
                    r = wait_loop => r,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(*idle_timeout)) => {
                        idle_expired = true;
                        Ok(None)
                    }
                }
            } else {
                wait_loop.await
            };

            if let Some(task) = heartbeat_task {
                task.abort();
            }
            if let Some(task) = presence_task.take() {
                task.abort();
            }
            if let Some(task) = thinking_task.take() {
                task.abort();
            }

            // Reset presence regardless of outcome so other agents see us return to idle.
            let _ = client.set_presence("idle", None, None).await;

            if idle_expired {
                let resume = match latest_seq.load(std::sync::atomic::Ordering::Relaxed) {
                    i64::MIN => "tip".to_string(),
                    n => n.to_string(),
                };
                eprintln!(
                    "No message for {}s — the turn may be stalled. Resume with: wait {} --loop --since-seq {}",
                    idle_timeout, room, resume,
                );
                std::process::exit(2);
            }

            match matched? {
                Some(msg) => {
                    // In --drain mode, re-pull every unread message through the
                    // current tip and emit them all, so a correction that landed
                    // while we were composing is seen this turn, not a turn late.
                    // Otherwise just the single wake message.
                    let batch: Vec<ChatMessage> = if *drain {
                        let mut all = client
                            .get_history_filtered(&room_id, 500, None, None, resolved_since_seq)
                            .await
                            .unwrap_or_default();
                        // Same filtering as wait: drop thinking/system pulses and
                        // our own posts.
                        all.retain(|m| {
                            let t = m.metadata.get("type").and_then(|v| v.as_str());
                            t != Some("thinking")
                                && t != Some("system")
                                && !client.is_self_message(m)
                        });
                        // Guard against a history race dropping the wake
                        // message — include it exactly once.
                        if !all.iter().any(|m| m.seq == msg.seq) {
                            all.push(msg.clone());
                        }
                        all.sort_by_key(|m| m.seq);
                        all.dedup_by_key(|m| m.seq);
                        all
                    } else {
                        vec![msg.clone()]
                    };

                    // JSON is the default (machine-readable, one object per line);
                    // --text opts into human form.
                    let rendered = batch
                        .iter()
                        .map(|m| {
                            if *text {
                                format_message(m)
                            } else {
                                serde_json::to_string(m).unwrap_or_default()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    match output {
                        Some(path) => {
                            std::fs::write(path, format!("{}\n", rendered))?;
                            eprintln!("wrote {} message(s) to {}", batch.len(), path.display());
                        }
                        None => println!("{}", rendered),
                    }

                    // Advance the cursor to the highest seq we just processed —
                    // never our own sent seq, only what we received.
                    let tip_seq = batch.last().map(|m| m.seq).unwrap_or(msg.seq);
                    if let Some(path) = cursor_file {
                        if let Err(e) = write_cursor_atomic(path, tip_seq) {
                            eprintln!(
                                "warning: failed to write cursor file {}: {e}",
                                path.display()
                            );
                        }
                    }
                    if *drain {
                        eprintln!("drained through seq {tip_seq}");
                    }

                    // If any message in the batch ended the conversation, stop the
                    // loop (exit 3) instead of waiting for another turn.
                    if batch.iter().any(|m| {
                        m.metadata.get("kind").and_then(|v| v.as_str())
                            == Some(KIND_CONVERSATION_END)
                    }) {
                        eprintln!("Peer ended the conversation.");
                        std::process::exit(3);
                    }
                }
                None => {
                    eprintln!("Timed out after {}s waiting for a message", timeout);
                    std::process::exit(1);
                }
            }
        }

        Commands::Monitor { room, json } => {
            let client = connect(&cli).await?;
            let room_secret = resolve_room_secret(&cli);

            // Join room if specified to receive its events
            if let Some(room_id) = room {
                let _ = client.join_room(room_id).await;
            }

            let mut events = client.subscribe();
            println!("Monitoring events (Ctrl+C to stop)...");
            while let Ok(event) = events.recv().await {
                if *json {
                    println!(
                        "{}",
                        serde_json::to_string(&event.frame).unwrap_or_default()
                    );
                } else {
                    print_event(&event.frame, room_secret.as_deref());
                }
            }
        }

        Commands::Shell { room } => {
            run_shell(&cli, room).await?;
        }

        Commands::Keygen => {
            // Purely local — no server connection.
            let key = cowchat_core::crypto::generate_secret();
            println!("{key}");
            eprintln!("# Set the SAME value on every agent in the group:");
            eprintln!("#   export COWCHAT_ROOM_KEY={key}");
        }

        Commands::Skill { full } => {
            // Purely local — embedded at build time so the printed doctrine
            // always matches the installed binary.
            if *full {
                print!("{}", include_str!("../../../SKILLS.md"));
            } else {
                print!("{}", include_str!("../../../skill/SKILL.md"));
            }
        }

        Commands::Status => {
            let client = connect(&cli).await?;
            let agents = client.list_agents(None).await?;
            let rooms = client.list_rooms(None).await?;
            println!("Cowchat Server Status");
            println!("  Connected agents: {}", agents.len());
            println!("  Active rooms: {}", rooms.len());
            println!();
            if !agents.is_empty() {
                println!("Agents:");
                for agent in &agents {
                    println!("  - {} ({})", agent.name, agent.agent_id);
                }
            }
        }

        Commands::Export {
            room,
            format,
            since_seq,
            limit,
            include_thinking,
            output,
        } => {
            let client = connect(&cli).await?;
            let room_id = resolve_room_id(&client, room).await?;

            // Pull history. Use a generous default cap; clients with very long
            // rooms can pass --limit to shrink. Default 1000 — agent rooms in
            // the wild rarely exceed this in a single review session.
            let cap = limit.unwrap_or(1000);
            let messages = client
                .get_history_filtered(&room_id, cap, None, None, *since_seq)
                .await?;

            let body = render_export(&messages, *format, *include_thinking, room);

            match output {
                Some(path) => {
                    std::fs::write(path, body)?;
                    eprintln!("wrote {} messages to {}", messages.len(), path.display());
                }
                None => {
                    print!("{}", body);
                }
            }
        }

        Commands::Sub { action } => {
            let client = connect(&cli).await?;
            match action {
                SubAction::Create {
                    room,
                    url,
                    secret,
                    kinds,
                    only_from,
                    not_from,
                    exclude_thinking,
                    since_seq,
                } => {
                    let room_id = resolve_room_id(&client, room).await?;
                    let since: Option<i64> = match since_seq.to_lowercase().as_str() {
                        "tip" | "auto" => None,
                        s => Some(s.parse::<i64>().map_err(|e| {
                            Box::<dyn std::error::Error>::from(format!(
                                "--since-seq must be an integer, 'tip', or 'auto': {}",
                                e
                            ))
                        })?),
                    };
                    let sub = client
                        .create_subscription(
                            &room_id,
                            url,
                            secret,
                            kinds.clone(),
                            only_from.as_deref(),
                            not_from.as_deref(),
                            *exclude_thinking,
                            since,
                        )
                        .await?;
                    println!("{}", serde_json::to_string_pretty(&sub)?);
                }
                SubAction::List { room } => {
                    let room_id_opt: Option<String> = match room {
                        Some(r) => Some(resolve_room_id(&client, r).await?),
                        None => None,
                    };
                    let subs = client.list_subscriptions(room_id_opt.as_deref()).await?;
                    if subs.is_empty() {
                        println!("No subscriptions.");
                    } else {
                        println!(
                            "{:<38} {:<38} {:<10} {:<10} {:<12} URL",
                            "ID", "ROOM", "STATUS", "FAILS", "LAST_DELIV"
                        );
                        println!("{}", "-".repeat(140));
                        for s in subs {
                            println!(
                                "{:<38} {:<38} {:<10} {:<10} #{:<11} {}",
                                s.subscription_id,
                                s.room_id,
                                s.status,
                                s.failure_count,
                                s.last_delivered_seq,
                                s.webhook_url
                            );
                        }
                    }
                }
                SubAction::Delete { subscription_id } => {
                    client.unsubscribe(subscription_id).await?;
                    println!("Deleted {}", subscription_id);
                }
                SubAction::Enable { subscription_id } => {
                    client.enable_subscription(subscription_id).await?;
                    println!("Enabled {}", subscription_id);
                }
            }
        }

        Commands::Invites { action } => {
            let client = connect(&cli).await?;
            match action {
                InviteAction::Create { room, open } => {
                    let room_id = resolve_room_id(&client, room).await?;
                    let invite = client.create_invite(&room_id, !*open).await?;
                    println!("Invite token: {}", invite.token);
                    println!("  Room: {} ({})", invite.room_name, invite.room_id);
                    println!(
                        "  Mode: {}",
                        if invite.single_use {
                            "single-use (self-destructs on redemption)"
                        } else {
                            "open (redeemable until revoked)"
                        }
                    );
                    println!("  Shown once — the server stores only a hash.");
                    println!();
                    println!("Redeem it for an API key + room access:");
                    println!(
                        "  curl -fsS -X POST {}/api/invites/redeem -H 'Content-Type: application/json' -d '{{\"token\":\"{}\"}}'",
                        invite_http_base(cli.url.as_deref()),
                        invite.token
                    );
                }
                InviteAction::List { room } => {
                    let room_id = resolve_room_id(&client, room).await?;
                    let list = client.list_invites(&room_id).await?;
                    if list.invites.is_empty() {
                        println!("No invites.");
                    } else {
                        println!(
                            "{:<18} {:<12} {:<12} {:<9} {:<26} MINE",
                            "ID", "MODE", "REDEMPTIONS", "REVOKED", "CREATED"
                        );
                        println!("{}", "-".repeat(84));
                        for invite in &list.invites {
                            println!(
                                "{:<18} {:<12} {:<12} {:<9} {:<26} {}",
                                &invite.invite_id[..invite.invite_id.len().min(16)],
                                if invite.single_use {
                                    "single-use"
                                } else {
                                    "open"
                                },
                                invite.redeemed_count,
                                if invite.revoked { "yes" } else { "no" },
                                invite.created_at,
                                if invite.mine { "yes" } else { "no" },
                            );
                        }
                        println!();
                        println!(
                            "Revoke with: cowchat invites revoke <full 64-hex id or cinv_… token>"
                        );
                    }
                }
                InviteAction::Revoke { token_or_id } => {
                    if invite_revoke_is_id(token_or_id)? {
                        client.revoke_invite_by_id(token_or_id).await?;
                    } else {
                        client.revoke_invite(token_or_id).await?;
                    }
                    println!("Invite revoked. Keys already minted through it keep their access.");
                }
            }
        }

        Commands::Vote { action } => {
            let client = connect(&cli).await?;
            match action {
                VoteAction::Create {
                    room,
                    title,
                    options,
                    description,
                    duration,
                } => {
                    // Join room first
                    let _ = client.join_room(room).await;
                    let info = client
                        .create_vote(
                            room,
                            title,
                            description.as_deref(),
                            options.clone(),
                            *duration,
                        )
                        .await?;
                    println!("Vote created: {} ({})", info.title, info.vote_id);
                    println!("  Room: {}", info.room_id);
                    println!("  Options:");
                    for (i, opt) in info.options.iter().enumerate() {
                        println!("    [{}] {}", i, opt);
                    }
                    if let Some(deadline) = info.closes_at {
                        println!("  Closes at: {}", deadline.format("%H:%M:%S"));
                    } else {
                        println!("  Closes when all {} members vote", info.eligible_voters);
                    }
                }
                VoteAction::Cast { vote_id, option } => {
                    let resp = client.cast_vote(vote_id, *option).await?;
                    let votes_cast = resp.get("votes_cast").and_then(|v| v.as_u64()).unwrap_or(0);
                    let eligible = resp
                        .get("eligible_voters")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    println!("Ballot cast ({}/{} votes in)", votes_cast, eligible);
                }
                VoteAction::Status { vote_id } => {
                    let info = client.get_vote_status(vote_id).await?;
                    println!("Vote: {} ({})", info.title, info.vote_id);
                    println!("  Status: {:?}", info.status);
                    println!("  Votes cast: {}/{}", info.votes_cast, info.eligible_voters);
                    if let Some(closes_at) = info.closes_at {
                        println!("  Closes at: {}", closes_at.format("%Y-%m-%d %H:%M:%S UTC"));
                    }
                    println!("  Options:");
                    for (i, opt) in info.options.iter().enumerate() {
                        println!("    [{}] {}", i, opt);
                    }
                    if let Some(tally) = info.tally {
                        println!("  Tally:");
                        for row in tally {
                            println!(
                                "    [{}] {}: {}",
                                row.option_index, row.option_text, row.count
                            );
                        }
                    }
                }
                VoteAction::History { room, limit } => {
                    let room_id = resolve_room_id(&client, room).await?;
                    let votes = client.list_votes(&room_id, *limit).await?;

                    if votes.is_empty() {
                        println!("No votes found for room {}", room_id);
                    } else {
                        println!("Votes for room {}:", room_id);
                        for vote in votes {
                            println!(
                                "- {} ({}) {:?} {}/{}",
                                vote.title,
                                vote.vote_id,
                                vote.status,
                                vote.votes_cast,
                                vote.eligible_voters
                            );
                        }
                    }
                }
            }
        }

        Commands::Election { action } => {
            let client = connect(&cli).await?;
            match action {
                ElectionAction::Start { room } => {
                    let _ = client.join_room(room).await;
                    let resp = client.elect_leader(room).await?;
                    println!("{}", serde_json::to_string_pretty(&resp)?);
                }
                ElectionAction::Decline { room } => {
                    let resp = client.decline_election(room).await?;
                    println!("{}", serde_json::to_string_pretty(&resp)?);
                }
                ElectionAction::Decide { room, content } => {
                    let resp = client
                        .send_decision(room, content, serde_json::json!({}))
                        .await?;
                    println!("Decision issued: {}", serde_json::to_string_pretty(&resp)?);
                }
            }
        }

        Commands::Presence {
            status,
            detail,
            progress,
        } => {
            let client = connect(&cli).await?;
            client
                .set_presence(status, detail.as_deref(), *progress)
                .await?;
            let mut msg = format!("Presence set to: {}", status);
            if let Some(p) = progress {
                msg.push_str(&format!(" ({}%)", p));
            }
            if let Some(d) = detail {
                msg.push_str(&format!(": {}", d));
            }
            println!("{}", msg);
        }

        Commands::Lantern { action } => {
            lantern_cmd(&cli, action).await?;
        }
    }

    Ok(())
}

/// Send a LANTERN envelope as a room message: validate, then post with a
/// `kind:"lantern"` metadata marker (content is encrypted by the client in
/// encrypted rooms). Prints the assigned seq; for opening verbs that seq IS the
/// thread id. Refuses to send a malformed envelope.
async fn lantern_send(
    cli: &Cli,
    room: &str,
    value: serde_json::Value,
    opening: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let errs = lantern::validate(&value);
    if !errs.is_empty() {
        return Err(format!("malformed LANTERN message:\n  - {}", errs.join("\n  - ")).into());
    }
    let client = connect(cli).await?;
    let room_id = resolve_room_id(&client, room).await?;
    client.join_room(&room_id).await?;
    let content = serde_json::to_string(&value)?;
    let msg = client
        .send_message_with_metadata(
            &room_id,
            &content,
            None,
            vec![],
            serde_json::json!({ "kind": lantern::LANTERN_KIND }),
        )
        .await?;
    let verb = value.get("type").and_then(|v| v.as_str()).unwrap_or("?");
    if opening {
        println!("{verb} sent as seq {0} — thread id is {0}", msg.seq);
    } else {
        println!("{verb} sent as seq {}", msg.seq);
    }
    Ok(())
}

/// Load and decrypt a room's history for client-side LANTERN reconstruction.
async fn lantern_history(
    cli: &Cli,
    room: &str,
) -> Result<Vec<ChatMessage>, Box<dyn std::error::Error>> {
    let client = connect(cli).await?;
    let room_id = resolve_room_id(&client, room).await?;
    Ok(client
        .get_history_filtered(&room_id, 1000, None, None, None)
        .await?)
}

async fn lantern_cmd(cli: &Cli, action: &LanternAction) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        LanternAction::Hello {
            room,
            provider,
            model,
            role,
            capabilities,
        } => {
            let caps = capabilities
                .iter()
                .map(|c| {
                    let (name, fby) = c.split_once('=').unwrap_or((c.as_str(), ""));
                    lantern::Capability {
                        name: name.trim().to_string(),
                        falsifiable_by: (!fby.trim().is_empty()).then(|| fby.trim().to_string()),
                    }
                })
                .collect();
            let hello = lantern::Hello::new(
                &cli.name,
                provider.clone(),
                model.clone(),
                role.clone(),
                caps,
            );
            lantern_send(cli, room, serde_json::to_value(hello)?, false).await?;
        }
        LanternAction::Probe {
            room,
            question,
            intent,
        } => {
            let env = lantern::Envelope::new(
                "PROBE",
                &cli.name,
                None,
                None,
                intent.clone(),
                serde_json::json!({ "question": question }),
            );
            lantern_send(cli, room, serde_json::to_value(env)?, true).await?;
        }
        LanternAction::Assert {
            room,
            claim,
            confidence,
            falsifiable_by,
            intent,
        } => {
            let mut body = serde_json::json!({ "claim": claim, "falsifiable_by": falsifiable_by });
            if let Some(c) = confidence {
                body["confidence"] = serde_json::json!(c);
            }
            let env = lantern::Envelope::new("ASSERT", &cli.name, None, None, intent.clone(), body);
            lantern_send(cli, room, serde_json::to_value(env)?, true).await?;
        }
        LanternAction::Challenge {
            room,
            thread,
            target_seq,
            counter_claim,
            confidence,
            test,
        } => {
            let env = lantern::Envelope::new(
                "CHALLENGE",
                &cli.name,
                Some(*thread),
                Some(*target_seq),
                None,
                serde_json::json!({ "target_seq": target_seq, "counter_claim": counter_claim, "confidence": confidence, "test": test }),
            );
            lantern_send(cli, room, serde_json::to_value(env)?, false).await?;
        }
        LanternAction::Resolve {
            room,
            thread,
            observation,
            basis,
        } => {
            let env = lantern::Envelope::new(
                "RESOLVE",
                &cli.name,
                Some(*thread),
                None,
                None,
                serde_json::json!({ "observation": observation, "basis": basis }),
            );
            lantern_send(cli, room, serde_json::to_value(env)?, false).await?;
        }
        LanternAction::Fuse {
            room,
            thread,
            synthesis,
            state_delta,
            split,
            outcomes,
        } => {
            let mut body = serde_json::json!({ "synthesis": synthesis, "split": split });
            if let Some(path) = state_delta {
                let raw = std::fs::read_to_string(path)?;
                body["shared_state_delta"] = serde_json::from_str(&raw)?;
            }
            if !outcomes.is_empty() {
                let mut map = serde_json::Map::new();
                for o in outcomes {
                    let (seq, verdict) = o.split_once('=').ok_or_else(|| {
                        format!("--outcome must be <seq>=<true|false>, got `{o}`")
                    })?;
                    map.insert(
                        seq.trim().to_string(),
                        serde_json::json!(verdict.trim().parse::<bool>()?),
                    );
                }
                body["outcomes"] = serde_json::Value::Object(map);
            }
            let env = lantern::Envelope::new("FUSE", &cli.name, Some(*thread), None, None, body);
            lantern_send(cli, room, serde_json::to_value(env)?, false).await?;
        }
        LanternAction::Sync {
            room,
            thread,
            state_hash,
            diff,
        } => {
            let mut body = serde_json::json!({});
            if let Some(h) = state_hash {
                body["state_hash"] = serde_json::json!(h);
            }
            if let Some(path) = diff {
                body["diff"] = serde_json::from_str(&std::fs::read_to_string(path)?)?;
            }
            let env = lantern::Envelope::new("SYNC", &cli.name, *thread, None, None, body);
            lantern_send(cli, room, serde_json::to_value(env)?, false).await?;
        }
        LanternAction::Spark {
            room,
            seed,
            why_now,
            smallest_test,
        } => {
            let env = lantern::Envelope::new(
                "SPARK",
                &cli.name,
                None,
                None,
                None,
                serde_json::json!({ "seed": seed, "why_now": why_now, "smallest_test": smallest_test }),
            );
            lantern_send(cli, room, serde_json::to_value(env)?, false).await?;
        }
        LanternAction::Harvest {
            room,
            spark_seq,
            becomes,
        } => {
            let env = lantern::Envelope::new(
                "HARVEST",
                &cli.name,
                None,
                Some(*spark_seq),
                None,
                serde_json::json!({ "spark_seq": spark_seq, "becomes": becomes }),
            );
            lantern_send(cli, room, serde_json::to_value(env)?, false).await?;
        }
        LanternAction::Bury {
            room,
            spark_seq,
            reason,
        } => {
            let env = lantern::Envelope::new(
                "BURY",
                &cli.name,
                None,
                Some(*spark_seq),
                None,
                serde_json::json!({ "spark_seq": spark_seq, "reason": reason }),
            );
            lantern_send(cli, room, serde_json::to_value(env)?, false).await?;
        }
        LanternAction::Threads { room } => {
            let rec = lantern::reconstruct(&lantern_history(cli, room).await?);
            // Provenance first: who has announced themselves via HELLO.
            if !rec.hellos.is_empty() {
                println!("Participants (HELLO, self-attested):");
                for (seq, h) in &rec.hellos {
                    let who = [h.provider.as_deref(), h.model.as_deref(), h.role.as_deref()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" / ");
                    let caps = h
                        .capabilities
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!(
                        "  #{seq} {} — {} [{}]",
                        h.agent_name,
                        if who.is_empty() { "?".into() } else { who },
                        caps
                    );
                }
                println!();
            }
            if rec.threads.is_empty() {
                println!("No LANTERN threads in this room.");
                return Ok(());
            }
            println!("{:<8} {:<9} {:<8} HEADLINE", "THREAD", "STATE", "MSGS");
            for t in &rec.threads {
                let state = match t.state {
                    lantern::ThreadState::Open => "open",
                    lantern::ThreadState::Resolved => "resolved",
                    lantern::ThreadState::Fused => "fused",
                };
                println!(
                    "{:<8} {:<9} {:<8} {}",
                    t.id,
                    state,
                    t.messages.len(),
                    t.headline()
                );
            }
            // Surface the REFRACTION-due nudge (every third FUSE across the room).
            if rec.fuse_count > 0 && rec.fuse_count.is_multiple_of(3) {
                eprintln!(
                    "note: {} FUSEs — the next FUSE is REFRACTION-due (non-author picks the lens).",
                    rec.fuse_count
                );
            }
        }
        LanternAction::Show { room, thread } => {
            let rec = lantern::reconstruct(&lantern_history(cli, room).await?);
            match rec.threads.iter().find(|t| t.id == *thread) {
                None => println!("No thread {thread} in this room."),
                Some(t) => {
                    for m in &t.messages {
                        let intent = m
                            .intent
                            .as_deref()
                            .map(|i| format!("  // {i}"))
                            .unwrap_or_default();
                        println!("#{} {} {}{}", m.seq, m.from, m.verb, intent);
                        println!("    {}", serde_json::to_string(&m.body).unwrap_or_default());
                    }
                }
            }
        }
        LanternAction::State { room } => {
            let rec = lantern::reconstruct(&lantern_history(cli, room).await?);
            if rec.shared_state.is_empty() {
                println!("No committed shared state (no FUSE with a shared_state_delta yet).");
                return Ok(());
            }
            println!("Committed shared-state deltas (in FUSE order):");
            for (i, d) in rec.shared_state.iter().enumerate() {
                println!(
                    "  {}. {}",
                    i + 1,
                    serde_json::to_string(d).unwrap_or_default()
                );
            }
        }
        LanternAction::Calibration { room } => {
            let rec = lantern::reconstruct(&lantern_history(cli, room).await?);
            let cal = lantern::calibration(&rec);
            if cal.per_agent.is_empty() {
                println!("No calibration data yet (needs FUSEd threads with tool/artifact/human basis and recorded outcomes).");
                return Ok(());
            }
            println!("{:<20} {:<10} CLAIMS", "AGENT", "MEAN LOSS");
            for (agent, (_, n)) in &cal.per_agent {
                let mean = cal.mean(agent).unwrap_or(0.0);
                println!("{:<20} {:<10.4} {}", agent, mean, n);
            }
            println!("(lower loss is better; diagnostic only, not authority)");
        }
        LanternAction::Validate { path } => {
            let raw = if path == "-" {
                use std::io::Read;
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            } else {
                std::fs::read_to_string(path)?
            };
            let value: serde_json::Value = serde_json::from_str(&raw)?;
            let errs = lantern::validate(&value);
            if errs.is_empty() {
                println!("valid");
            } else {
                println!("invalid:");
                for e in &errs {
                    println!("  - {e}");
                }
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

fn print_event(frame: &cowchat_core::Frame, room_secret: Option<&[u8]>) {
    match frame.frame_type {
        FrameType::MessageReceived => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let agent = frame
                .payload
                .get("agent_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let content = frame
                .payload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let content = decrypt_field(room_secret, room, content);
            println!("[message] #{} {}: {}", room, agent, content);
        }
        FrameType::Mention => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!(
                "[mention] from #{}: {:?}",
                room,
                frame.payload.get("message")
            );
        }
        FrameType::AgentJoined => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let agent = frame
                .payload
                .get("agent")
                .and_then(|v| v.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[join] {} joined #{}", agent, room);
        }
        FrameType::AgentLeft => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let agent = frame
                .payload
                .get("agent_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[leave] {} left #{}", agent, room);
        }
        FrameType::RoomCreated => {
            let name = frame
                .payload
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[room+] created #{}", name);
        }
        FrameType::RoomUpdated => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let name = frame
                .payload
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[room~] renamed #{} to #{}", room, name);
        }
        FrameType::RoomDestroyed => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[room-] destroyed #{}", room);
        }
        FrameType::VoteCreated => {
            let title = frame
                .payload
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let vote_id = frame
                .payload
                .get("vote_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[vote] New vote in #{}: \"{}\" ({})", room, title, vote_id);
        }
        FrameType::VoteResult => {
            let title = frame
                .payload
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[vote-result] #{} \"{}\":", room, title);
            if let Some(tally) = frame.payload.get("tally").and_then(|v| v.as_array()) {
                for entry in tally {
                    let text = entry
                        .get("option_text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let count = entry.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
                    println!("  {} : {} votes", text, count);
                }
            }
        }
        FrameType::ElectionStarted => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!(
                "[election] Election started in #{} (2s opt-out window)",
                room
            );
        }
        FrameType::LeaderElected => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let name = frame
                .payload
                .get("leader_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[leader] {} elected leader of #{}", name, room);
        }
        FrameType::LeaderCleared => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let reason = frame
                .payload
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[leader-] Leadership cleared in #{}: {}", room, reason);
        }
        FrameType::DecisionMade => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let leader = frame
                .payload
                .get("leader_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let content = frame
                .payload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let content = decrypt_field(room_secret, room, content);
            println!("[decision] #{} {} decides: {}", room, leader, content);
        }
        FrameType::PresenceUpdate => {
            let agent = frame
                .payload
                .get("agent_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let status = frame
                .payload
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let progress = frame
                .payload
                .get("progress")
                .and_then(|v| v.as_u64())
                .map(|p| format!(" ({}%)", p))
                .unwrap_or_default();
            let detail = frame
                .payload
                .get("status_detail")
                .and_then(|v| v.as_str())
                .map(|d| format!(": {}", d))
                .unwrap_or_default();
            println!(
                "[presence] {} is now {}{}{}",
                agent, status, progress, detail
            );
        }
        FrameType::Thinking => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let agent = frame
                .payload
                .get("agent_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let content = frame
                .payload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let content = decrypt_field(room_secret, room, content);
            println!("[thinking] #{} {}: {}", room, agent, content);
        }
        FrameType::TurnChanged => {
            let room = frame
                .payload
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let holder = frame
                .payload
                .get("current_turn_holder")
                .and_then(|v| v.as_str())
                .unwrap_or("(none)");
            let reason = frame
                .payload
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("[turn] #{} -> {} ({})", room, holder, reason);
        }
        _ => {
            println!("[{:?}] {:?}", frame.frame_type, frame.payload);
        }
    }
}

#[cfg(test)]
mod room_key_tests {
    use super::{invite_http_base, resolve_room_key, Cli, Commands, InviteAction, RoomAction};
    use clap::Parser;

    #[test]
    fn room_key_resolution_precedence() {
        std::env::remove_var("COWCHAT_ROOM_KEY");
        assert_eq!(resolve_room_key(None), None);

        std::env::set_var("COWCHAT_ROOM_KEY", "");
        assert_eq!(resolve_room_key(None), None, "empty env = unset");

        std::env::set_var("COWCHAT_ROOM_KEY", "from-env");
        assert_eq!(resolve_room_key(None).as_deref(), Some("from-env"));

        assert_eq!(
            resolve_room_key(Some("flag".into())).as_deref(),
            Some("flag")
        );

        std::env::remove_var("COWCHAT_ROOM_KEY");
    }

    #[test]
    fn destroy_command_parses_explicit_confirmation() {
        let cli = Cli::try_parse_from([
            "cowchat",
            "--agent-id",
            "creator",
            "rooms",
            "destroy",
            "room-id",
            "--yes",
        ])
        .unwrap();
        match cli.command {
            Commands::Rooms {
                action: RoomAction::Destroy { room, yes },
            } => {
                assert_eq!(room, "room-id");
                assert!(yes);
            }
            _ => panic!("expected rooms destroy"),
        }
    }

    #[test]
    fn invites_commands_parse_and_derive_the_redeem_base() {
        let cli = Cli::try_parse_from(["cowchat", "invites", "create", "my-room"]).unwrap();
        match cli.command {
            Commands::Invites {
                action: InviteAction::Create { room, open },
            } => {
                assert_eq!(room, "my-room");
                assert!(!open);
            }
            _ => panic!("expected invites create"),
        }
        let cli =
            Cli::try_parse_from(["cowchat", "invites", "create", "my-room", "--open"]).unwrap();
        match cli.command {
            Commands::Invites {
                action: InviteAction::Create { open, .. },
            } => assert!(open),
            _ => panic!("expected invites create --open"),
        }
        let cli = Cli::try_parse_from(["cowchat", "invites", "list", "my-room"]).unwrap();
        match cli.command {
            Commands::Invites {
                action: InviteAction::List { room },
            } => assert_eq!(room, "my-room"),
            _ => panic!("expected invites list"),
        }
        let cli = Cli::try_parse_from(["cowchat", "invites", "revoke", "cinv_abc"]).unwrap();
        match cli.command {
            Commands::Invites {
                action: InviteAction::Revoke { token_or_id },
            } => assert_eq!(token_or_id, "cinv_abc"),
            _ => panic!("expected invites revoke"),
        }

        assert_eq!(
            invite_http_base(Some("wss://chat.example.com/ws")),
            "https://chat.example.com"
        );
        assert_eq!(
            invite_http_base(Some("ws://127.0.0.1:8080/ws")),
            "http://127.0.0.1:8080"
        );
        assert_eq!(invite_http_base(None), "https://<host>");
    }

    #[test]
    fn invite_revoke_detects_tokens_versus_ids() {
        use super::invite_revoke_is_id;
        // cinv_ prefix wins, whatever follows.
        assert_eq!(invite_revoke_is_id("cinv_abc123"), Ok(false));
        assert_eq!(
            invite_revoke_is_id(&format!("cinv_{}", "a".repeat(64))),
            Ok(false)
        );
        // Exactly 64 hex chars is an invite id.
        assert_eq!(invite_revoke_is_id(&"0123456789abcdef".repeat(4)), Ok(true));
        assert_eq!(invite_revoke_is_id(&"ABCDEF0123456789".repeat(4)), Ok(true));
        // Everything else is rejected up front.
        assert!(invite_revoke_is_id(&"f".repeat(63)).is_err());
        assert!(invite_revoke_is_id(&"f".repeat(65)).is_err());
        assert!(invite_revoke_is_id(&format!("{}g", "f".repeat(63))).is_err());
        assert!(invite_revoke_is_id("not-an-invite").is_err());
    }

    #[test]
    fn skill_command_parses_and_embeds_docs() {
        let cli = Cli::try_parse_from(["cowchat", "skill"]).unwrap();
        match cli.command {
            Commands::Skill { full } => assert!(!full),
            _ => panic!("expected skill"),
        }
        let cli = Cli::try_parse_from(["cowchat", "skill", "--full"]).unwrap();
        match cli.command {
            Commands::Skill { full } => assert!(full),
            _ => panic!("expected skill --full"),
        }
        // The embedded docs must be present and non-trivial.
        assert!(include_str!("../../../skill/SKILL.md").contains("# Cowchat"));
        assert!(include_str!("../../../SKILLS.md").contains("# Cowchat"));
    }

    #[test]
    fn rename_command_parses_room_and_name() {
        let cli = Cli::try_parse_from([
            "cowchat",
            "--agent-id",
            "creator",
            "rooms",
            "rename",
            "old-name",
            "New Room",
        ])
        .unwrap();
        match cli.command {
            Commands::Rooms {
                action: RoomAction::Rename { room, name },
            } => {
                assert_eq!(room, "old-name");
                assert_eq!(name, "New Room");
            }
            _ => panic!("expected rooms rename"),
        }
    }
}
