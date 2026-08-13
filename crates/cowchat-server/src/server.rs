use cowchat_core::*;
use dashmap::DashMap;
use fs2::FileExt;
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener as StdUnixListener, UnixStream as StdUnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::{mpsc, oneshot};

use crate::auth;
use crate::broker::Broker;
use crate::connection::can_access_room;
use crate::connection::AgentConnection;
use crate::handler;
use crate::rate_limit::{RateLimiter, TierLimits};
use crate::reconnect::ReconnectManager;
use crate::store::Store;
use crate::tasks::TaskManager;
use crate::voting::VoteManager;

/// How often the server sends a ping to each connected agent.
const HEARTBEAT_INTERVAL_SECS: u64 = 30;
/// If no data received from an agent within this window, disconnect them.
const HEARTBEAT_TIMEOUT_SECS: u64 = 90;
const OUTBOUND_QUEUE_CAPACITY: usize = 256;
pub(crate) const MAX_FRAME_BYTES: usize = 1024 * 1024;
const REGISTER_TIMEOUT_SECS: u64 = 10;
/// How often the background task purges messages past their tier's retention.
const PURGE_INTERVAL: Duration = Duration::from_secs(3600);

fn tcp_connection_allows_keyless(allow_keyless_local: bool, peer_ip: std::net::IpAddr) -> bool {
    allow_keyless_local && peer_ip.is_loopback()
}

async fn read_frame_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<String>> {
    let mut bytes = Vec::with_capacity(4096);
    let read = reader
        .take((MAX_FRAME_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .await?;
    if read == 0 {
        return Ok(None);
    }
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "NDJSON frame exceeds 1 MiB limit",
        ));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

fn frame_room_id(frame: &Frame) -> Option<&str> {
    frame
        .payload
        .get("room_id")
        .and_then(|value| value.as_str())
        .or_else(|| {
            frame
                .payload
                .get("message")
                .and_then(|value| value.get("room_id"))
                .and_then(|value| value.as_str())
        })
}

fn accessible_room(room_id: &str, api_key: &str, no_auth: bool, store: &Store) -> Option<Room> {
    store
        .get_room(room_id)
        .ok()
        .flatten()
        .filter(|room| can_access_room(room, api_key, no_auth))
}

pub struct ServerConfig {
    pub socket_path: PathBuf,
    pub tcp_addr: Option<String>,
    pub http_addr: Option<String>,
    pub db_path: PathBuf,
    pub auth_key_path: PathBuf,
    pub no_auth: bool,
    /// Let same-machine clients use the UDS or loopback TCP without an API key.
    /// HTTP/WebSocket and non-loopback TCP connections are never covered.
    pub allow_keyless_local: bool,
    /// Explicit test/development escape hatch. Production defaults to false.
    pub allow_private_webhooks: bool,
    pub http_signup_enabled: bool,
    pub http_admin_secret: Option<String>,
    pub http_allowed_origins: Vec<String>,
    pub trusted_proxy_ips: Vec<std::net::IpAddr>,
}

pub struct CowchatServer {
    config: ServerConfig,
    _instance_lock: File,
    listeners: Mutex<Option<PreparedListeners>>,
    _socket_guard: OwnedSocketPath,
    broker: Arc<Broker>,
    store: Arc<Store>,
    vote_mgr: Arc<VoteManager>,
    rate_limiter: Arc<RateLimiter>,
    reconnect_mgr: Arc<ReconnectManager>,
    task_mgr: Arc<TaskManager>,
    webhook_mgr: Arc<crate::webhooks::WebhookManager>,
    api_key: String,
}

struct PreparedListeners {
    uds: StdUnixListener,
    tcp: Option<std::net::TcpListener>,
    http: Option<std::net::TcpListener>,
}

/// Removes only the exact socket inode created by this server. If another
/// process replaces the path, dropping this server must not unlink it.
struct OwnedSocketPath {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl OwnedSocketPath {
    fn new(path: PathBuf) -> io::Result<Self> {
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_socket() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("bound Unix socket path is not a socket: {}", path.display()),
            ));
        }
        Ok(Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    fn still_owns_path(&self) -> bool {
        std::fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.file_type().is_socket()
                && metadata.dev() == self.device
                && metadata.ino() == self.inode
        })
    }
}

impl Drop for OwnedSocketPath {
    fn drop(&mut self) {
        if self.still_owns_path() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn canonical_db_path(path: &Path) -> io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    match std::fs::symlink_metadata(&absolute) {
        Ok(_) => return std::fs::canonicalize(absolute),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let parent = absolute.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("database path has no parent: {}", absolute.display()),
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let canonical_parent = std::fs::canonicalize(parent)?;
    let file_name = absolute.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("database path has no file name: {}", absolute.display()),
        )
    })?;
    Ok(canonical_parent.join(file_name))
}

fn instance_lock_path(db_path: &Path) -> io::Result<PathBuf> {
    let file_name = db_path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("database path has no file name: {}", db_path.display()),
        )
    })?;
    let mut lock_name = file_name.to_os_string();
    lock_name.push(".lock");
    Ok(db_path.with_file_name(lock_name))
}

fn reject_hard_linked_database(db_path: &Path) -> io::Result<()> {
    let metadata = match std::fs::metadata(db_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.nlink() > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing to open Cowchat database with {} hard links: {}; use one canonical database path",
                metadata.nlink(),
                db_path.display()
            ),
        ));
    }
    Ok(())
}

fn acquire_instance_lock(db_path: &Path) -> io::Result<File> {
    let lock_path = instance_lock_path(db_path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).mode(0o600);
    let lock = options.open(&lock_path)?;
    lock.try_lock_exclusive().map_err(|error| {
        io::Error::new(
            if error.kind() == io::ErrorKind::WouldBlock {
                io::ErrorKind::AlreadyExists
            } else {
                error.kind()
            },
            format!(
                "another Cowchat server is already using database {}: {error}",
                db_path.display()
            ),
        )
    })?;
    crate::auth::harden_file_permissions(&lock_path)?;
    Ok(lock)
}

fn same_socket_inode(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    left.file_type().is_socket()
        && right.file_type().is_socket()
        && left.dev() == right.dev()
        && left.ino() == right.ino()
}

fn bind_unix_listener(path: &Path) -> io::Result<(StdUnixListener, OwnedSocketPath)> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    loop {
        match std::fs::symlink_metadata(path) {
            Ok(existing) => {
                if !existing.file_type().is_socket() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!(
                            "refusing to replace non-socket Unix listener path {}",
                            path.display()
                        ),
                    ));
                }

                match StdUnixStream::connect(path) {
                    Ok(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::AddrInUse,
                            format!(
                                "Unix socket is already accepting connections: {}",
                                path.display()
                            ),
                        ));
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                        ) =>
                    {
                        let current = match std::fs::symlink_metadata(path) {
                            Ok(current) => current,
                            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                            Err(error) => return Err(error),
                        };
                        if !same_socket_inode(&existing, &current) {
                            continue;
                        }
                        std::fs::remove_file(path)?;
                    }
                    Err(error) => {
                        return Err(io::Error::new(
                            error.kind(),
                            format!(
                                "refusing to replace Unix socket {} after connect failed: {error}",
                                path.display()
                            ),
                        ));
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }

        match StdUnixListener::bind(path) {
            Ok(listener) => {
                listener.set_nonblocking(true)?;
                let guard = OwnedSocketPath::new(path.to_path_buf())?;
                return Ok((listener, guard));
            }
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => continue,
            Err(error) => return Err(error),
        }
    }
}

fn bind_tcp_listener(addr: &str) -> io::Result<std::net::TcpListener> {
    let listener = std::net::TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

impl CowchatServer {
    pub fn new(mut config: ServerConfig) -> Result<Self, Box<dyn std::error::Error>> {
        // Lock the canonical database identity before touching SQLite, auth,
        // webhooks, or any listener path. A losing launch cannot migrate the
        // incumbent database or unlink its Unix socket.
        config.db_path = canonical_db_path(&config.db_path)?;
        // Path canonicalization collapses symlinks but cannot collapse hard
        // links. SQLite WAL/SHM files are path-derived, so allowing two aliases
        // of one inode would create independent lock domains over one database.
        reject_hard_linked_database(&config.db_path)?;
        let instance_lock = acquire_instance_lock(&config.db_path)?;

        // Reserve every configured endpoint before opening the database. This
        // keeps bind failures side-effect-free with respect to migrations,
        // authentication material, and background workers.
        let (uds_listener, socket_guard) = bind_unix_listener(&config.socket_path)?;
        let tcp_listener = config
            .tcp_addr
            .as_deref()
            .map(bind_tcp_listener)
            .transpose()?;
        let http_listener = config
            .http_addr
            .as_deref()
            .map(bind_tcp_listener)
            .transpose()?;

        let store = Arc::new(Store::open(&config.db_path)?);
        let agents: Arc<DashMap<String, AgentConnection>> = Arc::new(DashMap::new());
        let room_members: Arc<DashMap<String, Vec<String>>> = Arc::new(DashMap::new());
        let broker = Arc::new(Broker::new(agents, room_members));
        let vote_mgr = Arc::new(VoteManager::new(store.clone(), broker.clone()));
        let rate_limiter = Arc::new(RateLimiter::new());
        let reconnect_mgr = Arc::new(ReconnectManager::new());
        let task_mgr = Arc::new(TaskManager::new(store.clone()));
        let webhook_mgr = Arc::new(crate::webhooks::WebhookManager::new(
            store.clone(),
            config.allow_private_webhooks,
        ));
        let api_key = auth::load_or_create_key(&config.auth_key_path)?;

        log::info!("API key loaded from {:?}", config.auth_key_path);
        log::info!("Database at {:?}", config.db_path);

        Ok(Self {
            config,
            _instance_lock: instance_lock,
            listeners: Mutex::new(Some(PreparedListeners {
                uds: uds_listener,
                tcp: tcp_listener,
                http: http_listener,
            })),
            _socket_guard: socket_guard,
            broker,
            store,
            vote_mgr,
            rate_limiter,
            reconnect_mgr,
            task_mgr,
            webhook_mgr,
            api_key,
        })
    }

    /// Start the server, listening on UDS, TCP, and/or HTTP (as configured).
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        let prepared = self
            .listeners
            .lock()
            .map_err(|_| io::Error::other("prepared listener lock poisoned"))?
            .take()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::AlreadyExists, "server is already running")
            })?;
        let uds_listener = UnixListener::from_std(prepared.uds)?;
        let tcp_listener = prepared.tcp.map(TcpListener::from_std).transpose()?;
        let http_listener = prepared.http.map(TcpListener::from_std).transpose()?;

        // Spawn the webhook delivery worker. Held implicitly by the running task;
        // we don't await it here — when `run` returns the task is dropped along
        // with the server.
        let _webhook_worker = self.webhook_mgr.start();

        log::info!("Listening on UDS: {:?}", self.config.socket_path);

        // Spawn UDS accept loop as a task
        let uds_broker = self.broker.clone();
        let uds_store = self.store.clone();
        let uds_vote_mgr = self.vote_mgr.clone();
        let uds_api_key = self.api_key.clone();
        let uds_no_auth = self.config.no_auth;
        let uds_allow_keyless = self.config.allow_keyless_local;
        let uds_rate_limiter = self.rate_limiter.clone();
        let uds_reconnect_mgr = self.reconnect_mgr.clone();
        let uds_task_mgr = self.task_mgr.clone();
        let uds_webhook_mgr = self.webhook_mgr.clone();
        let uds_task = tokio::spawn(async move {
            loop {
                match uds_listener.accept().await {
                    Ok((stream, _addr)) => {
                        let (read_half, write_half) = tokio::io::split(stream);
                        let broker = uds_broker.clone();
                        let store = uds_store.clone();
                        let vote_mgr = uds_vote_mgr.clone();
                        let api_key = uds_api_key.clone();
                        let rate_limiter = uds_rate_limiter.clone();
                        let reconnect_mgr = uds_reconnect_mgr.clone();
                        let task_mgr = uds_task_mgr.clone();
                        let webhook_mgr = uds_webhook_mgr.clone();
                        tokio::spawn(async move {
                            let _ = connection_loop(
                                read_half,
                                write_half,
                                broker,
                                store,
                                vote_mgr,
                                api_key,
                                uds_no_auth,
                                uds_allow_keyless,
                                rate_limiter,
                                reconnect_mgr,
                                task_mgr,
                                webhook_mgr,
                                None,
                            )
                            .await;
                        });
                    }
                    Err(e) => {
                        log::error!("UDS accept error: {}", e);
                        break;
                    }
                }
            }
        });

        // Spawn TCP accept loop if configured
        let tcp_task = if let Some(tcp_listener) = tcp_listener {
            let addr = tcp_listener.local_addr()?;
            log::info!("Listening on TCP: {}", addr);
            let tcp_broker = self.broker.clone();
            let tcp_store = self.store.clone();
            let tcp_vote_mgr = self.vote_mgr.clone();
            let tcp_api_key = self.api_key.clone();
            let tcp_no_auth = self.config.no_auth;
            let tcp_allow_keyless_local = self.config.allow_keyless_local;
            let tcp_rate_limiter = self.rate_limiter.clone();
            let tcp_reconnect_mgr = self.reconnect_mgr.clone();
            let tcp_task_mgr = self.task_mgr.clone();
            let tcp_webhook_mgr = self.webhook_mgr.clone();
            Some(tokio::spawn(async move {
                loop {
                    match tcp_listener.accept().await {
                        Ok((stream, addr)) => {
                            log::info!("TCP connection from {}", addr);
                            // The peer address, not the bind address or a
                            // caller-supplied header, defines this trust edge.
                            let connection_allows_keyless =
                                tcp_connection_allows_keyless(tcp_allow_keyless_local, addr.ip());
                            let (read_half, write_half) = tokio::io::split(stream);
                            let broker = tcp_broker.clone();
                            let store = tcp_store.clone();
                            let vote_mgr = tcp_vote_mgr.clone();
                            let api_key = tcp_api_key.clone();
                            let rate_limiter = tcp_rate_limiter.clone();
                            let reconnect_mgr = tcp_reconnect_mgr.clone();
                            let task_mgr = tcp_task_mgr.clone();
                            let webhook_mgr = tcp_webhook_mgr.clone();
                            tokio::spawn(async move {
                                let _ = connection_loop(
                                    read_half,
                                    write_half,
                                    broker,
                                    store,
                                    vote_mgr,
                                    api_key,
                                    tcp_no_auth,
                                    connection_allows_keyless,
                                    rate_limiter,
                                    reconnect_mgr,
                                    task_mgr,
                                    webhook_mgr,
                                    None,
                                )
                                .await;
                            });
                        }
                        Err(e) => {
                            log::error!("TCP accept error: {}", e);
                            break;
                        }
                    }
                }
            }))
        } else {
            None
        };

        // Spawn HTTP/WebSocket listener if configured
        let http_task = if let Some(listener) = http_listener {
            let app_state = crate::web::AppState {
                broker: self.broker.clone(),
                store: self.store.clone(),
                vote_mgr: self.vote_mgr.clone(),
                rate_limiter: self.rate_limiter.clone(),
                no_auth: self.config.no_auth,
                api_key: self.api_key.clone(),
                reconnect_mgr: self.reconnect_mgr.clone(),
                task_mgr: self.task_mgr.clone(),
                webhook_mgr: self.webhook_mgr.clone(),
                signup_enabled: self.config.http_signup_enabled,
                admin_secret: self.config.http_admin_secret.clone(),
                allowed_origins: self.config.http_allowed_origins.clone(),
                trusted_proxy_ips: self.config.trusted_proxy_ips.clone(),
            };
            let router = crate::web::router(app_state);
            let addr = listener.local_addr()?;
            log::info!("HTTP/WebSocket listening on {}", addr);
            Some(tokio::spawn(async move {
                if let Err(e) = axum::serve(
                    listener,
                    router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
                )
                .await
                {
                    log::error!("HTTP server error: {}", e);
                }
            }))
        } else {
            None
        };

        // Background retention purge: periodically delete messages past each
        // tier's retention window so the DB doesn't grow without bound. Tiers
        // with no window (None = enterprise) are never queried, so their data is
        // kept. Runs once on start, then every PURGE_INTERVAL.
        let purge_store = self.store.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(PURGE_INTERVAL);
            loop {
                tick.tick().await;
                let mut total = 0usize;
                for (tier, limits) in [("free", TierLimits::free()), ("pro", TierLimits::pro())] {
                    if let Some(days) = limits.history_retention_days {
                        let modifier = format!("-{days} days");
                        match purge_store.purge_messages_by_tier(tier, &modifier) {
                            Ok(n) => total += n,
                            Err(e) => log::warn!("retention purge ({tier}) failed: {e}"),
                        }
                    }
                }
                // No VACUUM: SQLite reuses the freed pages, so a fixed retention
                // window bounds file growth on its own. A full VACUUM would hold
                // the single store mutex through a whole-DB rebuild — a server-
                // wide stall we don't want on an interval.
                if total > 0 {
                    log::info!("retention purge: deleted {total} expired message(s)");
                }
            }
        });

        // Wait for shutdown signal
        tokio::select! {
            _ = uds_task => {},
            _ = async { if let Some(t) = tcp_task { t.await.ok(); } else { std::future::pending::<()>().await } } => {},
            _ = async { if let Some(t) = http_task { t.await.ok(); } else { std::future::pending::<()>().await } } => {},
            signal = shutdown_signal() => {
                signal?;
                log::info!("Shutting down...");
            }
        }

        Ok(())
    }

    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    pub fn socket_path(&self) -> &Path {
        &self.config.socket_path
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    pub fn broker(&self) -> &Arc<Broker> {
        &self.broker
    }

    pub fn rate_limiter(&self) -> &Arc<RateLimiter> {
        &self.rate_limiter
    }

    pub fn reconnect_mgr(&self) -> &Arc<ReconnectManager> {
        &self.reconnect_mgr
    }

    pub fn task_mgr(&self) -> &Arc<TaskManager> {
        &self.task_mgr
    }
}

#[cfg(unix)]
async fn shutdown_signal() -> io::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> io::Result<()> {
    tokio::signal::ctrl_c().await
}

async fn wait_for_transport_disconnect(disconnect: Option<&Arc<tokio::sync::Notify>>) {
    match disconnect {
        Some(disconnect) => disconnect.notified().await,
        None => std::future::pending::<()>().await,
    }
}

/// Main per-connection loop. Handles registration then dispatches frames.
#[allow(clippy::too_many_arguments)]
pub async fn connection_loop<R, W>(
    read_half: R,
    mut write_half: W,
    broker: Arc<Broker>,
    store: Arc<Store>,
    vote_mgr: Arc<VoteManager>,
    api_key: String,
    // Server-wide `--no-auth`; unlike `allow_keyless`, this intentionally
    // bypasses room ownership boundaries.
    no_auth: bool,
    // This transport is local and may register with an empty key. Supplying a
    // non-empty key still requires normal validation.
    allow_keyless: bool,
    rate_limiter: Arc<RateLimiter>,
    reconnect_mgr: Arc<ReconnectManager>,
    task_mgr: Arc<TaskManager>,
    webhook_mgr: Arc<crate::webhooks::WebhookManager>,
    transport_disconnect: Option<Arc<tokio::sync::Notify>>,
) -> Result<(), Box<dyn std::error::Error>>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut reader = BufReader::new(read_half);
    // Phase 1: Wait for register command
    let (
        agent_id,
        agent_name,
        agent_capabilities,
        session_id,
        agent_api_key,
        reconnected_rooms,
        takeover_rooms,
        register_reply_to,
        mut missed_messages,
        reclaim_lease,
        agent_lifecycle_guard,
    ) = loop {
        let line = tokio::time::timeout(
            Duration::from_secs(REGISTER_TIMEOUT_SECS),
            read_frame_line(&mut reader),
        )
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "registration timed out")
        })??;
        let Some(line) = line else {
            return Ok(()); // Connection closed before registration
        };

        let frame = match Frame::from_line(&line) {
            Ok(f) => f,
            Err(e) => {
                let err_frame = Frame::error(
                    None,
                    ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
                );
                write_half
                    .write_all(err_frame.to_line()?.as_bytes())
                    .await?;
                continue;
            }
        };

        if frame.frame_type == FrameType::Ping {
            let pong = Frame::pong(frame.id.as_deref());
            write_half.write_all(pong.to_line()?.as_bytes()).await?;
            continue;
        }

        if frame.frame_type != FrameType::Register {
            let err = Frame::error(
                frame.id.as_deref(),
                ErrorPayload::new(ErrorCode::NotRegistered, "Must register first"),
            );
            write_half.write_all(err.to_line()?.as_bytes()).await?;
            continue;
        }

        let payload: RegisterPayload = match serde_json::from_value(frame.payload) {
            Ok(p) => p,
            Err(e) => {
                let err = Frame::error(
                    frame.id.as_deref(),
                    ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
                );
                write_half.write_all(err.to_line()?.as_bytes()).await?;
                continue;
            }
        };

        // Reject clients whose wire protocol is outside our supported range, with
        // a clear "upgrade" signal instead of a later confusing parse error. An
        // absent version is a pre-versioning client, treated as v1. A mismatch is
        // a build mismatch the client can't fix on this connection, so we close it.
        let client_protocol = payload.protocol_version.unwrap_or(1);
        if !(MIN_SUPPORTED_PROTOCOL..=PROTOCOL_VERSION).contains(&client_protocol) {
            let msg = if client_protocol > PROTOCOL_VERSION {
                format!(
                    "Client protocol v{client_protocol} is newer than this server (v{PROTOCOL_VERSION}); the server needs an upgrade"
                )
            } else {
                format!(
                    "Client protocol v{client_protocol} is no longer supported (server requires >= v{MIN_SUPPORTED_PROTOCOL}); run `brew upgrade cowchat`"
                )
            };
            let err = Frame::error(
                frame.id.as_deref(),
                ErrorPayload::new(ErrorCode::UnsupportedProtocol, msg),
            );
            write_half.write_all(err.to_line()?.as_bytes()).await?;
            return Ok(());
        }

        // Validate API key
        let authenticated_key = if no_auth
            || (allow_keyless && payload.key.is_empty())
            || (!payload.key.is_empty()
                && (payload.key == api_key
                    || store.validate_api_key(&payload.key).unwrap_or(false)))
        {
            payload.key.clone()
        } else {
            let err = Frame::error(
                frame.id.as_deref(),
                ErrorPayload::new(ErrorCode::Unauthorized, "Invalid API key"),
            );
            write_half.write_all(err.to_line()?.as_bytes()).await?;
            return Ok(());
        };

        // Check rate limit for agent count (skip in no_auth mode)
        if !no_auth && !authenticated_key.is_empty() {
            let tier = store
                .get_key_tier(&authenticated_key)
                .unwrap_or_else(|_| "free".to_string());
            let limits = TierLimits::for_tier(&tier);
            if !rate_limiter.check_agent_limit(&authenticated_key, &limits) {
                let err = Frame::error(
                    frame.id.as_deref(),
                    ErrorPayload::new(
                        ErrorCode::RateLimitAgents,
                        "Agent limit exceeded for this API key",
                    ),
                );
                write_half.write_all(err.to_line()?.as_bytes()).await?;
                return Ok(());
            }
        }

        let stable_identity_requested = payload.agent_id.is_some();
        let agent_id = payload
            .agent_id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        if !no_auth && !authenticated_key.is_empty() && stable_identity_requested {
            match store.claim_agent_identity(&agent_id, &authenticated_key) {
                Ok(true) => {}
                Ok(false) => {
                    let err = Frame::error(
                        frame.id.as_deref(),
                        ErrorPayload::new(
                            ErrorCode::AgentIdTaken,
                            "Agent ID belongs to a different API key",
                        ),
                    );
                    write_half.write_all(err.to_line()?.as_bytes()).await?;
                    continue;
                }
                Err(error) => {
                    let err = Frame::error(
                        frame.id.as_deref(),
                        ErrorPayload::new(ErrorCode::InternalError, error.to_string()),
                    );
                    write_half.write_all(err.to_line()?.as_bytes()).await?;
                    continue;
                }
            }
        }

        // Same-ID takeover decisions and live installation are serialized
        // against disconnect cleanup. Without this guard, an old session can
        // pass its ownership check immediately before a new session installs,
        // then erase the new session's membership during cleanup.
        let agent_lifecycle_guard = broker.lock_agent_lifecycle();

        // === Reconnect logic ===
        // Claim a stash without removing it. It remains bufferable until the
        // live connection is installed and room membership is restored.
        let mut reconnected_rooms: Option<HashSet<String>> = None;
        let mut takeover_rooms: Option<HashSet<String>> = None;
        let mut missed_messages: Vec<Frame> = Vec::new();
        let mut reclaim_lease = None;

        if payload.reconnect {
            match reconnect_mgr.begin_reclaim(&agent_id, &authenticated_key, no_auth) {
                Ok(Some(lease)) => {
                    let stashed = lease
                        .snapshot()
                        .expect("a newly acquired reconnect lease has a stash");
                    let mut rooms = stashed.rooms;
                    rooms.retain(|room_id| {
                        accessible_room(room_id, &authenticated_key, no_auth, &store).is_some()
                    });
                    reconnected_rooms = Some(rooms);
                    missed_messages = stashed.missed_messages;
                    missed_messages.retain(|frame| {
                        frame_room_id(frame)
                            .map(|room_id| {
                                accessible_room(room_id, &authenticated_key, no_auth, &store)
                                    .is_some()
                            })
                            .unwrap_or(true)
                    });
                    reclaim_lease = Some(lease);
                }
                Ok(None) => {}
                Err(crate::reconnect::ReclaimError::CredentialMismatch) => {
                    drop(agent_lifecycle_guard);
                    let err = Frame::error(
                        frame.id.as_deref(),
                        ErrorPayload::new(
                            ErrorCode::AgentIdTaken,
                            "Agent ID belongs to a different API key",
                        ),
                    );
                    write_half.write_all(err.to_line()?.as_bytes()).await?;
                    continue;
                }
                Err(crate::reconnect::ReclaimError::AlreadyClaimed) => {
                    drop(agent_lifecycle_guard);
                    let err = Frame::error(
                        frame.id.as_deref(),
                        ErrorPayload::new(
                            ErrorCode::AgentIdTaken,
                            "Reconnect already in progress for this agent ID",
                        ),
                    );
                    write_half.write_all(err.to_line()?.as_bytes()).await?;
                    continue;
                }
            }
        }

        if reclaim_lease.is_none() {
            if let Some(existing) = broker.agents.get(&agent_id) {
                if payload.reconnect {
                    if !no_auth && existing.api_key != authenticated_key {
                        drop(existing);
                        drop(agent_lifecycle_guard);
                        let err = Frame::error(
                            frame.id.as_deref(),
                            ErrorPayload::new(
                                ErrorCode::AgentIdTaken,
                                "Agent ID belongs to a different API key",
                            ),
                        );
                        write_half.write_all(err.to_line()?.as_bytes()).await?;
                        continue;
                    }
                    // Take over a still-live connection with the same agent_id. Covers
                    // the race where a prior one-shot call's disconnect hasn't been
                    // processed yet — the newest connection wins. We inherit the old
                    // connection's rooms; replacing its broker entry below drops the
                    // old sender (ending its send task), and the old reader's later
                    // cleanup is a no-op because it checks session ownership.
                    drop(existing);
                    takeover_rooms = Some(
                        broker
                            .rooms_for_agent(&agent_id)
                            .into_iter()
                            .filter(|room_id| {
                                accessible_room(room_id, &authenticated_key, no_auth, &store)
                                    .is_some()
                            })
                            .collect(),
                    );
                } else {
                    // Still "connected" and not a reconnect — reject.
                    drop(existing);
                    drop(agent_lifecycle_guard);
                    let err = Frame::error(
                        frame.id.as_deref(),
                        ErrorPayload::new(ErrorCode::AgentIdTaken, "Agent ID already in use"),
                    );
                    write_half.write_all(err.to_line()?.as_bytes()).await?;
                    continue;
                }
            }
        }

        // Track agent in rate limiter
        if !authenticated_key.is_empty() {
            rate_limiter.add_agent(&authenticated_key);
        }

        log::info!(
            "Agent registered: {} ({}) capabilities={:?}{}",
            payload.name,
            agent_id,
            payload.capabilities,
            if reconnected_rooms.is_some() {
                " [RECONNECTED]"
            } else {
                ""
            },
        );

        // Record session
        let session_id = uuid::Uuid::new_v4().to_string();
        let _ = store.record_session_start(
            &session_id,
            &agent_id,
            &payload.name,
            &payload.capabilities,
        );

        break (
            agent_id,
            payload.name,
            payload.capabilities,
            session_id,
            authenticated_key,
            reconnected_rooms,
            takeover_rooms,
            frame.id,
            missed_messages,
            reclaim_lease,
            agent_lifecycle_guard,
        );
    };

    // Phase 2: Set up channel + task pair
    let (tx, mut rx) = mpsc::channel::<Frame>(OUTBOUND_QUEUE_CAPACITY);
    let (initial_tx, initial_rx) = oneshot::channel::<(Vec<Frame>, oneshot::Sender<bool>)>();
    let disconnect = Arc::new(tokio::sync::Notify::new());

    // The connection becomes live before registration is acknowledged. Events
    // produced in that window queue in `rx`, while this gate guarantees the OK
    // response and reconnect replay are written first.
    let send_task = tokio::spawn(async move {
        let Ok((initial_frames, initial_ack)) = initial_rx.await else {
            return;
        };
        let mut initial_write_ok = true;
        for frame in initial_frames {
            let Ok(line) = frame.to_line() else {
                continue;
            };
            if write_half.write_all(line.as_bytes()).await.is_err() {
                initial_write_ok = false;
                break;
            }
        }
        let _ = initial_ack.send(initial_write_ok);
        if !initial_write_ok {
            return;
        }
        while let Some(frame) = rx.recv().await {
            match frame.to_line() {
                Ok(line) => {
                    if write_half.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    log::error!("Frame serialization error: {}", e);
                }
            }
        }
    });

    let agent_info = AgentInfo {
        agent_id: agent_id.clone(),
        name: agent_name.clone(),
        capabilities: agent_capabilities.clone(),
        connected_at: Some(chrono::Utc::now()),
        last_active: Some(chrono::Utc::now()),
        status: Some("idle".into()),
        status_detail: None,
        progress: None,
    };

    // Store the connection
    let conn = AgentConnection::new(
        agent_info,
        session_id.clone(),
        tx.clone(),
        send_task,
        tokio::spawn(async {}),
        disconnect.clone(),
        agent_api_key.clone(),
    );
    broker.agents.insert(agent_id.clone(), conn);
    let was_reconnected = reconnected_rooms.is_some();
    let mut restored_rooms = HashSet::new();

    // If we took over a still-live connection, the agent is still in its rooms
    // (we never left them). Restore its room set on the new connection and
    // re-assert membership idempotently — no rejoin broadcast, since peers
    // never saw it leave.
    if let Some(rooms) = takeover_rooms {
        for room_id in &rooms {
            let _ = broker.join_room(&agent_id, room_id, || {
                accessible_room(room_id, &agent_api_key, no_auth, &store).is_some()
            });
        }
    }

    // Restore room memberships if reconnecting
    if let Some(rooms) = reconnected_rooms {
        for room_id in &rooms {
            if broker
                .join_room(&agent_id, room_id, || {
                    accessible_room(room_id, &agent_api_key, no_auth, &store).is_some()
                })
                .is_err()
            {
                continue;
            }

            // Broadcast rejoin event
            let event = Frame::event(
                FrameType::AgentJoined,
                serde_json::json!({
                    "room_id": room_id,
                    "agent": {
                        "agent_id": agent_id,
                        "name": agent_name,
                    },
                    "reconnected": true,
                }),
            );
            broker.broadcast_to_room(room_id, &agent_id, &event);
            restored_rooms.insert(room_id.clone());
        }
    }
    drop(agent_lifecycle_guard);

    // Refresh the pre-ack snapshot while the reclaim lease remains active.
    // Any events that arrive during the actual write are collected as a tail
    // after the write succeeds; a failed write drops the lease and preserves
    // the stash for another reconnect attempt.
    if let Some(lease) = reclaim_lease.as_ref() {
        if let Some(stashed) = lease.snapshot() {
            missed_messages = stashed.missed_messages;
        }
    }
    missed_messages.retain(|frame| {
        frame_room_id(frame)
            .map(|room_id| accessible_room(room_id, &agent_api_key, no_auth, &store).is_some())
            .unwrap_or(true)
    });
    restored_rooms.retain(|room_id| {
        broker.is_agent_in_room(&agent_id, room_id)
            && accessible_room(room_id, &agent_api_key, no_auth, &store).is_some()
    });

    let mut ok_payload = serde_json::json!({
        "agent_id": agent_id,
        "name": agent_name,
        "protocol_version": PROTOCOL_VERSION,
    });
    if was_reconnected {
        let mut room_list: Vec<String> = restored_rooms.into_iter().collect();
        room_list.sort();
        ok_payload["reconnected"] = serde_json::json!(true);
        ok_payload["rooms"] = serde_json::json!(room_list);
        ok_payload["missed_messages"] = serde_json::json!(missed_messages.len());
    }
    let mut initial_frames = Vec::with_capacity(missed_messages.len() + 1);
    initial_frames.push(Frame::ok(register_reply_to.as_deref(), ok_payload));
    let replayed_ids: HashSet<String> = missed_messages
        .iter()
        .filter_map(|frame| frame.id.clone())
        .collect();
    initial_frames.extend(missed_messages.iter().cloned());
    let (initial_ack_tx, initial_ack_rx) = oneshot::channel();
    let initial_queued = initial_tx.send((initial_frames, initial_ack_tx)).is_ok();
    let initial_written = if initial_queued {
        matches!(
            tokio::time::timeout(Duration::from_secs(5), initial_ack_rx).await,
            Ok(Ok(true))
        )
    } else {
        false
    };
    if !initial_written {
        log::warn!("Registration writer ended before acknowledging {agent_id}");
        let failed_registration_guard = broker.lock_agent_lifecycle();
        let removed_own_connection = broker
            .agents
            .remove_if(&agent_id, |_, connection| {
                connection.session_id == session_id
            })
            .is_some();
        if removed_own_connection {
            for room_id in broker.rooms_for_agent(&agent_id) {
                broker.leave_room(&agent_id, &room_id);
            }
        }
        drop(failed_registration_guard);
        if !agent_api_key.is_empty() {
            rate_limiter.remove_agent(&agent_api_key);
        }
        let _ = store.record_session_end(&session_id);
        return Ok(());
    }

    if let Some(lease) = reclaim_lease {
        if let Some(stashed) = lease.commit() {
            for frame in stashed.missed_messages {
                if frame
                    .id
                    .as_ref()
                    .is_some_and(|id| replayed_ids.contains(id))
                {
                    continue;
                }
                if frame_room_id(&frame).is_some_and(|room_id| {
                    accessible_room(room_id, &agent_api_key, no_auth, &store).is_none()
                }) {
                    continue;
                }
                if tx.send(frame).await.is_err() {
                    break;
                }
            }
        }
    }

    // Phase 3: Read and process frames (with heartbeat)
    let heartbeat_tx = tx.clone();
    let heartbeat_agent_id = agent_id.clone();
    let heartbeat_broker = broker.clone();
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
        loop {
            interval.tick().await;
            // Only send heartbeat if the agent is still connected
            if !heartbeat_broker.agents.contains_key(&heartbeat_agent_id) {
                break;
            }
            let ping = Frame {
                id: Some(uuid::Uuid::new_v4().to_string()),
                reply_to: None,
                frame_type: FrameType::Ping,
                payload: serde_json::json!({"heartbeat": true}),
            };
            if heartbeat_tx.try_send(ping).is_err() {
                break;
            }
        }
    });

    let mut last_activity = std::time::Instant::now();

    loop {
        let read_result = tokio::select! {
            biased;
            _ = wait_for_transport_disconnect(transport_disconnect.as_ref()) => break,
            _ = disconnect.notified() => break,
            result = tokio::time::timeout(
                Duration::from_secs(HEARTBEAT_TIMEOUT_SECS),
                read_frame_line(&mut reader),
            ) => result,
        };

        match read_result {
            Ok(Ok(None)) => break, // Connection closed
            Ok(Ok(Some(line))) => {
                last_activity = std::time::Instant::now();

                let frame = match Frame::from_line(&line) {
                    Ok(f) => f,
                    Err(e) => {
                        let err = Frame::error(
                            None,
                            ErrorPayload::new(ErrorCode::InvalidPayload, e.to_string()),
                        );
                        if !matches!(
                            tokio::time::timeout(Duration::from_secs(5), tx.send(err)).await,
                            Ok(Ok(()))
                        ) {
                            break;
                        }
                        continue;
                    }
                };

                // Pong responses don't need processing
                if frame.frame_type == FrameType::Pong {
                    continue;
                }

                // Subscribe performs its only suspending external operation (URL
                // validation) before mutating state, so it is safe to cancel at
                // transport close. Other commands may intentionally finish an
                // atomic state transition before disconnect cleanup begins.
                let cancel_during_webhook_validation = frame.frame_type == FrameType::Subscribe;
                let response_future = handler::handle_frame(
                    frame,
                    &agent_id,
                    &agent_name,
                    &broker,
                    &store,
                    &vote_mgr,
                    &agent_api_key,
                    &rate_limiter,
                    no_auth,
                    &task_mgr,
                    &webhook_mgr,
                    &reconnect_mgr,
                );
                tokio::pin!(response_future);
                let response = if cancel_during_webhook_validation {
                    tokio::select! {
                        biased;
                        _ = wait_for_transport_disconnect(transport_disconnect.as_ref()) => break,
                        _ = disconnect.notified() => break,
                        response = &mut response_future => response,
                    }
                } else {
                    response_future.await
                };
                if !matches!(
                    tokio::time::timeout(Duration::from_secs(5), tx.send(response)).await,
                    Ok(Ok(()))
                ) {
                    log::warn!("Outbound queue stalled for {} ({})", agent_name, agent_id);
                    break;
                }
            }
            Ok(Err(e)) => {
                log::warn!(
                    "Read error for {} ({}), treating as disconnect: {}",
                    agent_name,
                    agent_id,
                    e
                );
                break;
            }
            Err(_) => {
                // Timeout — no data received within HEARTBEAT_TIMEOUT_SECS
                log::warn!(
                    "Heartbeat timeout for {} ({}) — last activity {:.0}s ago",
                    agent_name,
                    agent_id,
                    last_activity.elapsed().as_secs_f64()
                );
                break;
            }
        }
    }

    // Stop heartbeat task
    heartbeat_task.abort();

    // Phase 4: Cleanup on disconnect
    log::info!("Agent disconnected: {} ({})", agent_name, agent_id);

    // Remove from rate limiter
    if !agent_api_key.is_empty() {
        rate_limiter.remove_agent(&agent_api_key);
    }

    let cleanup_lifecycle_guard = broker.lock_agent_lifecycle();

    // If a newer connection took over this agent_id (take-over / reconnect race),
    // it now owns the rooms and registry entry — don't tear them down. Just end
    // our own session record and exit.
    let still_mine = broker
        .agents
        .get(&agent_id)
        .map(|c| c.session_id == session_id)
        .unwrap_or(false);
    if !still_mine {
        drop(cleanup_lifecycle_guard);
        let _ = store.record_session_end(&session_id);
        return Ok(());
    }

    // Collect room memberships BEFORE leaving them (for stash)
    let agent_rooms: HashSet<String> = broker.rooms_for_agent(&agent_id).into_iter().collect();

    // Stash for reconnect
    if !agent_rooms.is_empty() {
        reconnect_mgr.stash(
            agent_id.clone(),
            agent_name.clone(),
            agent_api_key.clone(),
            agent_rooms.clone(),
        );
    }

    // Leave all rooms
    for room_id in &agent_rooms {
        let outcome = broker.leave_room(&agent_id, room_id);

        // Broadcast leave event
        let event = Frame::event(
            FrameType::AgentLeft,
            serde_json::json!({
                "room_id": room_id,
                "agent_id": agent_id,
            }),
        );
        broker.broadcast_to_room_all(room_id, &event);

        // Buffer the leave event for stashed agents in this room
        for stashed_id in reconnect_mgr.stashed_members_of_room(room_id) {
            if stashed_id != agent_id {
                reconnect_mgr.buffer_message(&stashed_id, event.clone());
            }
        }

        // If this agent held the turn token, the broker already advanced it.
        // Tell the remaining members so they know whose turn it is now.
        if outcome.holder_changed && !outcome.now_empty {
            crate::handler::broadcast_turn_changed(&broker, room_id, "disconnected");
        }
    }

    // Clear leadership if this agent was a leader
    vote_mgr.clear_leader_if_agent(&agent_id, &broker);

    // Remove agent connection
    broker.agents.remove_if(&agent_id, |_, connection| {
        connection.session_id == session_id
    });
    drop(cleanup_lifecycle_guard);

    // Record session end
    let _ = store.record_session_end(&session_id);

    Ok(())
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    fn config(db_path: PathBuf, socket_path: PathBuf, auth_key_path: PathBuf) -> ServerConfig {
        ServerConfig {
            socket_path,
            tcp_addr: None,
            http_addr: None,
            db_path,
            auth_key_path,
            no_auth: false,
            allow_keyless_local: true,
            allow_private_webhooks: false,
            http_signup_enabled: false,
            http_admin_secret: None,
            http_allowed_origins: vec![],
            trusted_proxy_ips: vec![],
        }
    }

    fn socket_identity(path: &Path) -> (u64, u64) {
        let metadata = std::fs::symlink_metadata(path).unwrap();
        assert!(metadata.file_type().is_socket());
        (metadata.dev(), metadata.ino())
    }

    #[tokio::test]
    async fn same_database_loser_cannot_touch_incumbent_socket_or_auth() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("cowchat.db");
        let socket_path = temp.path().join("cowchat.sock");
        let first_key_path = temp.path().join("first-auth.key");
        let losing_key_path = temp.path().join("losing-auth.key");
        let first =
            CowchatServer::new(config(db_path.clone(), socket_path.clone(), first_key_path))
                .unwrap();
        let incumbent_identity = socket_identity(&socket_path);

        let error = match CowchatServer::new(config(
            db_path,
            socket_path.clone(),
            losing_key_path.clone(),
        )) {
            Ok(_) => panic!("a second server using the same database must be rejected"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("already using database"));
        assert!(
            !losing_key_path.exists(),
            "loser must not create auth state"
        );
        assert_eq!(socket_identity(&socket_path), incumbent_identity);
        StdUnixStream::connect(&socket_path).expect("incumbent socket must remain live");
        drop(first);
        assert!(!socket_path.exists());
    }

    #[tokio::test]
    async fn canonical_database_aliases_share_one_instance_lock() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let alias_dir = temp.path().join("data-alias");
        std::os::unix::fs::symlink(&data_dir, &alias_dir).unwrap();
        let first_socket = temp.path().join("first.sock");
        let second_socket = temp.path().join("second.sock");
        let first = CowchatServer::new(config(
            data_dir.join("cowchat.db"),
            first_socket,
            temp.path().join("first.key"),
        ))
        .unwrap();

        let error = match CowchatServer::new(config(
            alias_dir.join("cowchat.db"),
            second_socket.clone(),
            temp.path().join("second.key"),
        )) {
            Ok(_) => panic!("canonical aliases of one database must share a lock"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("already using database"));
        assert!(
            !second_socket.exists(),
            "canonical lock rejection must happen before socket binding"
        );
        drop(first);
    }

    #[tokio::test]
    async fn hard_linked_database_alias_is_rejected_before_startup_side_effects() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("cowchat.db");
        let first_socket = temp.path().join("first.sock");
        let first = CowchatServer::new(config(
            db_path.clone(),
            first_socket.clone(),
            temp.path().join("first.key"),
        ))
        .unwrap();
        let incumbent_identity = socket_identity(&first_socket);

        let alias_path = temp.path().join("cowchat-alias.db");
        std::fs::hard_link(&db_path, &alias_path).unwrap();
        assert_eq!(std::fs::metadata(&db_path).unwrap().nlink(), 2);
        let losing_socket = temp.path().join("losing.sock");
        let losing_key = temp.path().join("losing.key");
        let losing_lock = instance_lock_path(&alias_path).unwrap();

        let error = match CowchatServer::new(config(
            alias_path.clone(),
            losing_socket.clone(),
            losing_key.clone(),
        )) {
            Ok(_) => panic!("a hard-linked database alias must be rejected"),
            Err(error) => error,
        };

        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(error.to_string().contains("database with 2 hard links"));
        assert!(!losing_lock.exists(), "loser must not create a lock file");
        assert!(!losing_socket.exists(), "loser must not bind its socket");
        assert!(!losing_key.exists(), "loser must not create auth state");
        assert!(
            !alias_path.with_extension("db-wal").exists(),
            "loser must not create an alias WAL"
        );
        assert!(
            !alias_path.with_extension("db-shm").exists(),
            "loser must not create alias shared-memory state"
        );
        assert_eq!(socket_identity(&first_socket), incumbent_identity);
        StdUnixStream::connect(&first_socket).expect("incumbent socket must remain live");

        drop(first);
        assert!(!first_socket.exists());
    }

    #[test]
    fn live_unix_socket_is_preserved_before_database_or_auth_open() {
        let temp = tempfile::tempdir().unwrap();
        let socket_path = temp.path().join("occupied.sock");
        let incumbent = StdUnixListener::bind(&socket_path).unwrap();
        incumbent.set_nonblocking(true).unwrap();
        let incumbent_identity = socket_identity(&socket_path);
        let db_path = temp.path().join("must-not-exist.db");
        let key_path = temp.path().join("must-not-exist.key");

        let error = match CowchatServer::new(config(
            db_path.clone(),
            socket_path.clone(),
            key_path.clone(),
        )) {
            Ok(_) => panic!("a live Unix socket must not be replaced"),
            Err(error) => error,
        };

        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::AddrInUse
        );
        assert_eq!(socket_identity(&socket_path), incumbent_identity);
        incumbent
            .accept()
            .expect("startup probe must have reached the incumbent listener");
        assert!(
            !db_path.exists(),
            "listener failure must precede SQLite open"
        );
        assert!(
            !key_path.exists(),
            "listener failure must precede auth open"
        );
    }

    #[test]
    fn non_socket_unix_path_is_preserved_before_database_or_auth_open() {
        let temp = tempfile::tempdir().unwrap();
        let socket_path = temp.path().join("occupied.sock");
        std::fs::write(&socket_path, b"keep me").unwrap();
        let metadata = std::fs::metadata(&socket_path).unwrap();
        let db_path = temp.path().join("must-not-exist.db");
        let key_path = temp.path().join("must-not-exist.key");

        let error = match CowchatServer::new(config(
            db_path.clone(),
            socket_path.clone(),
            key_path.clone(),
        )) {
            Ok(_) => panic!("a non-socket listener path must not be replaced"),
            Err(error) => error,
        };

        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&socket_path).unwrap(), b"keep me");
        assert_eq!(
            std::fs::metadata(&socket_path).unwrap().ino(),
            metadata.ino()
        );
        assert!(
            !db_path.exists(),
            "listener failure must precede SQLite open"
        );
        assert!(
            !key_path.exists(),
            "listener failure must precede auth open"
        );
    }

    #[test]
    fn occupied_tcp_listener_fails_before_database_open_and_cleans_own_uds() {
        let temp = tempfile::tempdir().unwrap();
        let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut server_config = config(
            temp.path().join("must-not-exist.db"),
            temp.path().join("candidate.sock"),
            temp.path().join("must-not-exist.key"),
        );
        server_config.tcp_addr = Some(occupied.local_addr().unwrap().to_string());
        let db_path = server_config.db_path.clone();
        let key_path = server_config.auth_key_path.clone();
        let socket_path = server_config.socket_path.clone();

        assert!(CowchatServer::new(server_config).is_err());
        assert!(
            !db_path.exists(),
            "TCP bind failure must precede SQLite open"
        );
        assert!(
            !key_path.exists(),
            "TCP bind failure must precede auth open"
        );
        assert!(
            !socket_path.exists(),
            "failed construction must remove only its own Unix socket"
        );
    }

    #[tokio::test]
    async fn stale_unix_socket_is_replaced_and_owned_cleanup_is_exact() {
        let temp = tempfile::tempdir().unwrap();
        let socket_path = temp.path().join("stale.sock");
        let stale = StdUnixListener::bind(&socket_path).unwrap();
        let stale_identity = socket_identity(&socket_path);
        drop(stale);

        let server = CowchatServer::new(config(
            temp.path().join("cowchat.db"),
            socket_path.clone(),
            temp.path().join("auth.key"),
        ))
        .unwrap();
        assert_ne!(socket_identity(&socket_path), stale_identity);
        StdUnixStream::connect(&socket_path).expect("replacement socket must be live");
        drop(server);
        assert!(!socket_path.exists());
    }

    #[tokio::test]
    async fn dropping_server_does_not_unlink_a_replacement_socket() {
        let temp = tempfile::tempdir().unwrap();
        let socket_path = temp.path().join("replace.sock");
        let server = CowchatServer::new(config(
            temp.path().join("cowchat.db"),
            socket_path.clone(),
            temp.path().join("auth.key"),
        ))
        .unwrap();

        std::fs::remove_file(&socket_path).unwrap();
        let replacement = StdUnixListener::bind(&socket_path).unwrap();
        let replacement_identity = socket_identity(&socket_path);
        drop(server);

        assert_eq!(socket_identity(&socket_path), replacement_identity);
        drop(replacement);
    }
}

#[cfg(test)]
mod local_auth_tests {
    use super::*;

    #[test]
    fn keyless_local_never_covers_non_loopback_tcp() {
        assert!(tcp_connection_allows_keyless(
            true,
            "127.0.0.1".parse().unwrap()
        ));
        assert!(tcp_connection_allows_keyless(true, "::1".parse().unwrap()));
        assert!(!tcp_connection_allows_keyless(
            true,
            "192.0.2.10".parse().unwrap()
        ));
        assert!(!tcp_connection_allows_keyless(
            false,
            "127.0.0.1".parse().unwrap()
        ));
    }

    #[tokio::test]
    async fn destroy_during_pending_registration_is_not_silently_reported_restored() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        store
            .create_room_with_visibility(
                "pending-room",
                "Pending Room",
                None,
                None,
                Some("stable"),
                "private",
                Some("owner-key"),
                false,
            )
            .unwrap();
        let agents = Arc::new(DashMap::new());
        let broker = Arc::new(Broker::new(agents.clone(), Arc::new(DashMap::new())));
        let (sender, mut events) = mpsc::channel(4);
        agents.insert(
            "stable".into(),
            AgentConnection::new(
                AgentInfo {
                    agent_id: "stable".into(),
                    name: "Stable".into(),
                    capabilities: vec![],
                    connected_at: None,
                    last_active: None,
                    status: None,
                    status_detail: None,
                    progress: None,
                },
                "pending-session".into(),
                sender,
                tokio::spawn(async {}),
                tokio::spawn(async {}),
                Arc::new(tokio::sync::Notify::new()),
                "owner-key".into(),
            ),
        );
        broker
            .join_room("stable", "pending-room", || {
                store.get_room("pending-room").unwrap().is_some()
            })
            .unwrap();

        let (room, _) = broker
            .destroy_room_with("pending-room", || {
                store.destroy_room_authorized("pending-room", "stable", "owner-key", false)
            })
            .unwrap();
        let destroyed = Frame::event(
            FrameType::RoomDestroyed,
            serde_json::json!({"room_id": "pending-room"}),
        );
        crate::handler::broadcast_room_destroyed(&broker, &room, false, &destroyed);

        let would_report_restored = broker.is_agent_in_room("stable", "pending-room")
            && accessible_room("pending-room", "owner-key", false, &store).is_some();
        assert!(!would_report_restored);
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.frame_type, FrameType::RoomDestroyed);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disconnect_cleanup_cannot_remove_a_newer_takeover_session() {
        fn connection(session_id: &str) -> AgentConnection {
            let (sender, _receiver) = mpsc::channel(1);
            AgentConnection::new(
                AgentInfo {
                    agent_id: "stable".into(),
                    name: "Stable".into(),
                    capabilities: vec![],
                    connected_at: None,
                    last_active: None,
                    status: None,
                    status_detail: None,
                    progress: None,
                },
                session_id.into(),
                sender,
                tokio::spawn(async {}),
                tokio::spawn(async {}),
                Arc::new(tokio::sync::Notify::new()),
                "owner-key".into(),
            )
        }

        let agents = Arc::new(DashMap::new());
        let broker = Arc::new(Broker::new(agents.clone(), Arc::new(DashMap::new())));
        agents.insert("stable".into(), connection("old-session"));
        let newer = connection("new-session");

        // Old cleanup wins the serialization gate first. The takeover cannot
        // install until that removal finishes, so it cannot be erased by a
        // stale post-check cleanup.
        let cleanup_guard = broker.lock_agent_lifecycle();
        let takeover_broker = broker.clone();
        let takeover_agents = agents.clone();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _takeover_guard = takeover_broker.lock_agent_lifecycle();
            takeover_agents.insert("stable".into(), newer);
            done_tx.send(()).unwrap();
        });
        assert!(agents
            .remove_if("stable", |_, connection| {
                connection.session_id == "old-session"
            })
            .is_some());
        drop(cleanup_guard);
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            agents.get("stable").unwrap().session_id,
            "new-session",
            "newer takeover must survive old-session cleanup"
        );
    }
}
