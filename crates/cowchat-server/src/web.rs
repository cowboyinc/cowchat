use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, Path, State,
    },
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use rust_embed::Embed;
use std::sync::Arc;
use std::{net::IpAddr, net::SocketAddr};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

use crate::broker::Broker;
use crate::rate_limit::RateLimiter;
use crate::reconnect::ReconnectManager;
use crate::server::connection_loop;
use crate::store::Store;
use crate::tasks::TaskManager;
use crate::voting::VoteManager;

const ADMIN_HEADER: &str = "x-cowchat-admin";
const API_KEY_HEADER: &str = "x-cowchat-key";
const WEBSOCKET_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// Hyper may cancel an upgraded handler while its spawned bridge tasks are
/// still live. Dropping this guard closes both halves of the pipe and wakes the
/// detached connection loop so it can run registered-agent cleanup.
struct WebSocketConnectionGuard {
    transport_disconnect: Arc<tokio::sync::Notify>,
    input_abort: tokio::task::AbortHandle,
    output_abort: tokio::task::AbortHandle,
    armed: bool,
}

impl WebSocketConnectionGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for WebSocketConnectionGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.input_abort.abort();
        self.output_abort.abort();
        self.transport_disconnect.notify_one();
    }
}

#[derive(Embed)]
#[folder = "web/"]
struct WebAssets;

/// Shared state passed to all axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub broker: Arc<Broker>,
    pub store: Arc<Store>,
    pub vote_mgr: Arc<VoteManager>,
    pub rate_limiter: Arc<RateLimiter>,
    pub no_auth: bool,
    pub api_key: String,
    pub reconnect_mgr: Arc<ReconnectManager>,
    pub task_mgr: Arc<TaskManager>,
    pub webhook_mgr: Arc<crate::webhooks::WebhookManager>,
    pub signup_enabled: bool,
    pub admin_secret: Option<String>,
    pub allowed_origins: Vec<String>,
    pub trusted_proxy_ips: Vec<IpAddr>,
}

pub fn router(state: AppState) -> Router {
    let allowed_origins = state.allowed_origins.clone();
    let router = Router::new()
        .route("/ws", get(ws_handler))
        .route("/api/keys", post(create_api_key))
        .route("/api/status", get(api_status))
        .route("/api/rooms", get(api_list_rooms))
        .route("/api/agents", get(api_list_agents))
        .route("/api/rooms/{room_id}/history", get(api_room_history))
        .fallback(static_handler)
        .with_state(state);
    let origins = allowed_origins
        .iter()
        .filter_map(|origin| HeaderValue::from_str(origin).ok())
        .collect::<Vec<_>>();
    if origins.is_empty() {
        router
    } else {
        router.layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(origins)
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([
                    header::CONTENT_TYPE,
                    header::HeaderName::from_static(ADMIN_HEADER),
                    header::HeaderName::from_static(API_KEY_HEADER),
                ]),
        )
    }
}

// --- WebSocket handler ---

fn origin_allowed(headers: &HeaderMap, allowed: &[String]) -> bool {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };
    allowed.iter().any(|candidate| candidate == origin)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    if !origin_allowed(&headers, &state.allowed_origins) {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.on_upgrade(move |socket| handle_ws_connection(socket, state))
        .into_response()
}

async fn handle_ws_connection(ws: WebSocket, state: AppState) {
    let (mut ws_sender, mut ws_receiver) = ws.split();

    // Create an in-memory duplex stream (bidirectional pipe)
    let (server_stream, client_stream) = tokio::io::duplex(65536);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let (client_read, mut client_write) = tokio::io::split(client_stream);

    // Task 1: WebSocket receiver → pipe writer
    // Reads text messages from WS, writes them as NDJSON lines to the pipe
    let mut ws_to_pipe = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_receiver.next().await {
            match msg {
                Message::Text(text) => {
                    let text_bytes = text.as_bytes();
                    if text_bytes.len() > crate::server::MAX_FRAME_BYTES {
                        break;
                    }
                    if client_write.write_all(text_bytes).await.is_err() {
                        break;
                    }
                    if !text_bytes.ends_with(b"\n") && client_write.write_all(b"\n").await.is_err()
                    {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {} // Ignore binary, ping/pong (axum handles pong automatically)
            }
        }
        // Drop client_write to signal EOF to server_read
    });

    // Task 2: Pipe reader → WebSocket sender
    // Reads NDJSON lines from the pipe, sends them as WS text messages
    let mut pipe_to_ws = tokio::spawn(async move {
        let mut reader = tokio::io::BufReader::new(client_read);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let trimmed = line.trim_end().to_string();
                    if ws_sender.send(Message::Text(trimmed.into())).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = ws_sender.close().await;
    });

    // Task 3: Run the existing connection_loop on the server side of the duplex
    let broker = state.broker;
    let store = state.store;
    let vote_mgr = state.vote_mgr;
    let rate_limiter = state.rate_limiter;
    let api_key = state.api_key;
    let no_auth = state.no_auth;
    let reconnect_mgr = state.reconnect_mgr;
    let task_mgr = state.task_mgr;
    let webhook_mgr = state.webhook_mgr;
    let transport_disconnect = Arc::new(tokio::sync::Notify::new());
    let connection_disconnect = transport_disconnect.clone();

    let mut connection_task = tokio::spawn(async move {
        let _ = connection_loop(
            server_read,
            server_write,
            broker,
            store,
            vote_mgr,
            api_key,
            no_auth,
            false,
            rate_limiter,
            reconnect_mgr,
            task_mgr,
            webhook_mgr,
            Some(connection_disconnect),
        )
        .await;
    });
    let mut connection_guard = WebSocketConnectionGuard {
        transport_disconnect: transport_disconnect.clone(),
        input_abort: ws_to_pipe.abort_handle(),
        output_abort: pipe_to_ws.abort_handle(),
        armed: true,
    };

    enum CompletedTask {
        WebSocketInput,
        WebSocketOutput,
        Connection,
    }

    let completed = tokio::select! {
        _ = &mut ws_to_pipe => CompletedTask::WebSocketInput,
        _ = &mut pipe_to_ws => CompletedTask::WebSocketOutput,
        _ = &mut connection_task => CompletedTask::Connection,
    };

    match completed {
        CompletedTask::Connection => {
            // connection_loop can write a terminal error and return immediately.
            // Keep the WebSocket sender alive until it drains that final frame and
            // emits a close; returning here first can strand or discard the error.
            if tokio::time::timeout(WEBSOCKET_DRAIN_TIMEOUT, &mut pipe_to_ws)
                .await
                .is_err()
            {
                pipe_to_ws.abort();
                let _ = pipe_to_ws.await;
            }
            ws_to_pipe.abort();
            let _ = ws_to_pipe.await;
        }
        CompletedTask::WebSocketInput => {
            // Dropping client_write at the end of ws_to_pipe delivers EOF to
            // connection_loop. Also signal transport cancellation so an
            // in-flight command (for example DNS validation) is cancelled and
            // phase-4 cleanup cannot be held open by external I/O.
            transport_disconnect.notify_one();
            let _ = connection_task.await;
            if tokio::time::timeout(WEBSOCKET_DRAIN_TIMEOUT, &mut pipe_to_ws)
                .await
                .is_err()
            {
                pipe_to_ws.abort();
                let _ = pipe_to_ws.await;
            }
        }
        CompletedTask::WebSocketOutput => {
            // Stop receiving so client_write is dropped, then let
            // connection_loop observe EOF and perform registered-agent cleanup.
            ws_to_pipe.abort();
            let _ = ws_to_pipe.await;
            transport_disconnect.notify_one();
            let _ = connection_task.await;
        }
    }
    connection_guard.disarm();
}

// --- REST API endpoints ---

async fn create_api_key(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    if !state.signup_enabled {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "HTTP signup is disabled"})),
        );
    }
    // With no admin secret configured, signup is open self-serve; the per-IP
    // rate limit below is the only brake on key minting.
    if let Some(admin_secret) = state.admin_secret.as_deref() {
        let supplied_admin = headers
            .get(ADMIN_HEADER)
            .and_then(|value| value.to_str().ok());
        if supplied_admin != Some(admin_secret) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "admin secret required"})),
            );
        }
    }
    let client_ip = signup_bucket(peer, &headers, &state.trusted_proxy_ips);
    if !state.rate_limiter.try_register_signup(&client_ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": "signup rate limit exceeded for your IP; try again later"
            })),
        );
    }

    let label = body.and_then(|b| b.get("label").and_then(|v| v.as_str()).map(String::from));

    let key = crate::auth::generate_key();

    if let Err(e) = state.store.create_api_key(&key, label.as_deref()) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        );
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "api_key": key,
            "tier": "free",
        })),
    )
}

fn signup_bucket(peer: SocketAddr, headers: &HeaderMap, trusted_proxies: &[IpAddr]) -> String {
    if trusted_proxies.contains(&peer.ip()) {
        if let Some(value) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit(',').next())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
        {
            return value.to_string();
        }
    }
    peer.ip().to_string()
}

async fn api_status(State(state): State<AppState>) -> impl IntoResponse {
    let agent_count = state.broker.agents.len();
    let rooms = state.store.list_rooms(None).unwrap_or_default();

    Json(serde_json::json!({
        "status": "ok",
        "agents_connected": agent_count,
        "rooms": rooms.len(),
    }))
}

async fn api_list_rooms(State(state): State<AppState>) -> impl IntoResponse {
    // Public API: only show public rooms
    let rooms = state
        .store
        .list_rooms_for_key(None, None)
        .unwrap_or_default();

    Json(serde_json::json!({"rooms": rooms}))
}

async fn api_list_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    let Some(key) = headers
        .get(API_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if key != state.api_key && !state.store.validate_api_key(key).unwrap_or(false) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let agents: Vec<serde_json::Value> = state
        .broker
        .agents
        .iter()
        .filter(|agent| state.no_auth || agent.api_key == key)
        .map(|a| {
            serde_json::json!({
                "agent_id": a.info.agent_id,
                "name": a.info.name,
                "capabilities": a.info.capabilities,
            })
        })
        .collect();

    Json(serde_json::json!({"agents": agents})).into_response()
}

async fn api_room_history(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> impl IntoResponse {
    // Only allow history for public rooms via REST API
    let room = state.store.get_room(&room_id).ok().flatten();
    match room {
        Some(r) if r.visibility == "public" => {
            let messages = state
                .store
                .get_history(&room_id, 100, None)
                .unwrap_or_default();
            (
                StatusCode::OK,
                Json(serde_json::json!({"messages": messages})),
            )
                .into_response()
        }
        Some(_) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Room is private"})),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Room not found"})),
        )
            .into_response(),
    }
}

// --- Static file serving ---

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match WebAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                content.data.into_owned(),
            )
                .into_response()
        }
        None => {
            // SPA fallback: serve index.html for unmatched routes
            match WebAssets::get("index.html") {
                Some(content) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html".to_string())],
                    content.data.into_owned(),
                )
                    .into_response(),
                None => StatusCode::NOT_FOUND.into_response(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cowchat_core::{ErrorCode, ErrorPayload, Frame, FrameType, RegisterPayload};
    use dashmap::DashMap;
    use tokio_tungstenite::{connect_async, tungstenite::Message as ClientMessage};

    fn test_state() -> AppState {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(Broker::new(
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
        ));
        AppState {
            broker: broker.clone(),
            store: store.clone(),
            vote_mgr: Arc::new(VoteManager::new(store.clone(), broker)),
            rate_limiter: Arc::new(RateLimiter::new()),
            no_auth: false,
            api_key: "master".into(),
            reconnect_mgr: Arc::new(ReconnectManager::new()),
            task_mgr: Arc::new(TaskManager::new(store.clone())),
            webhook_mgr: Arc::new(crate::webhooks::WebhookManager::new(store, true)),
            signup_enabled: false,
            admin_secret: None,
            allowed_origins: vec![],
            trusted_proxy_ips: vec![],
        }
    }

    async fn start_test_web_server(state: AppState) -> (tokio::task::JoinHandle<()>, SocketAddr) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = router(state);
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        (server, addr)
    }

    #[tokio::test]
    async fn websocket_delivers_terminal_registration_error_before_closing() {
        let (server, addr) = start_test_web_server(test_state()).await;
        let (mut socket, _) = connect_async(format!("ws://{addr}/ws")).await.unwrap();
        let register = Frame {
            id: Some("invalid-registration".into()),
            reply_to: None,
            frame_type: FrameType::Register,
            payload: serde_json::to_value(RegisterPayload {
                key: "wrong-key".into(),
                agent_id: None,
                name: "invalid-agent".into(),
                capabilities: vec![],
                reconnect: false,
                protocol_version: Some(cowchat_core::PROTOCOL_VERSION),
            })
            .unwrap(),
        };
        socket
            .send(ClientMessage::Text(
                register.to_line().unwrap().trim_end().to_owned().into(),
            ))
            .await
            .unwrap();

        let message = tokio::time::timeout(std::time::Duration::from_secs(1), socket.next())
            .await
            .expect("server should answer an invalid registration before closing")
            .expect("server closed before delivering the registration error")
            .expect("WebSocket read should succeed");
        let ClientMessage::Text(text) = message else {
            panic!("expected registration error text before close, got {message:?}");
        };
        let response = Frame::from_line(&text).unwrap();
        let error: ErrorPayload = serde_json::from_value(response.payload).unwrap();
        assert_eq!(response.frame_type, FrameType::Error);
        assert_eq!(response.reply_to.as_deref(), Some("invalid-registration"));
        assert_eq!(error.code, ErrorCode::Unauthorized);
        assert_eq!(error.message, "Invalid API key");

        let close = tokio::time::timeout(std::time::Duration::from_secs(1), socket.next())
            .await
            .expect("server should close after a terminal registration error");
        assert!(
            matches!(close, Some(Ok(ClientMessage::Close(_))) | None),
            "registration error must precede connection close, got {close:?}"
        );

        server.abort();
    }

    #[tokio::test]
    async fn registered_websocket_close_cleans_up_agent_and_room_membership() {
        let state = test_state();
        let observed_state = state.clone();
        let (server, addr) = start_test_web_server(state).await;
        let (mut socket, _) = connect_async(format!("ws://{addr}/ws")).await.unwrap();
        let agent_id = "websocket-cleanup-agent";
        let register = Frame {
            id: Some("register-cleanup-agent".into()),
            reply_to: None,
            frame_type: FrameType::Register,
            payload: serde_json::to_value(RegisterPayload {
                key: "master".into(),
                agent_id: Some(agent_id.into()),
                name: "cleanup-agent".into(),
                capabilities: vec![],
                reconnect: false,
                protocol_version: Some(cowchat_core::PROTOCOL_VERSION),
            })
            .unwrap(),
        };
        socket
            .send(ClientMessage::Text(
                register.to_line().unwrap().trim_end().to_owned().into(),
            ))
            .await
            .unwrap();
        let registered = next_text_frame(&mut socket).await;
        assert_eq!(registered.frame_type, FrameType::Ok);
        assert_eq!(
            registered.reply_to.as_deref(),
            Some("register-cleanup-agent")
        );

        let join = Frame {
            id: Some("join-lobby".into()),
            reply_to: None,
            frame_type: FrameType::JoinRoom,
            payload: serde_json::json!({"room_id": "lobby"}),
        };
        socket
            .send(ClientMessage::Text(
                join.to_line().unwrap().trim_end().to_owned().into(),
            ))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !observed_state.broker.is_agent_in_room(agent_id, "lobby") {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("registered WebSocket should join the lobby");
        assert!(observed_state.broker.agents.contains_key(agent_id));

        socket.send(ClientMessage::Close(None)).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while observed_state.broker.agents.contains_key(agent_id)
                || observed_state.broker.is_agent_in_room(agent_id, "lobby")
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("registered WebSocket close should finish server-side cleanup");

        server.abort();
    }

    #[tokio::test]
    async fn websocket_close_cancels_inflight_command_before_cleanup() {
        let validation_started = Arc::new(tokio::sync::Notify::new());
        let validation_release = Arc::new(tokio::sync::Notify::new());
        let mut state = test_state();
        state.webhook_mgr = Arc::new(crate::webhooks::WebhookManager::new_with_validation_gate(
            state.store.clone(),
            true,
            validation_started.clone(),
            validation_release,
        ));
        let observed_state = state.clone();
        let (server, addr) = start_test_web_server(state).await;
        let (mut socket, _) = connect_async(format!("ws://{addr}/ws")).await.unwrap();
        let agent_id = "websocket-cancel-agent";

        let register = Frame {
            id: Some("register-cancel-agent".into()),
            reply_to: None,
            frame_type: FrameType::Register,
            payload: serde_json::to_value(RegisterPayload {
                key: "master".into(),
                agent_id: Some(agent_id.into()),
                name: "cancel-agent".into(),
                capabilities: vec![],
                reconnect: false,
                protocol_version: Some(cowchat_core::PROTOCOL_VERSION),
            })
            .unwrap(),
        };
        socket
            .send(ClientMessage::Text(
                register.to_line().unwrap().trim_end().to_owned().into(),
            ))
            .await
            .unwrap();
        assert_eq!(next_text_frame(&mut socket).await.frame_type, FrameType::Ok);

        let join = Frame {
            id: Some("join-before-cancel".into()),
            reply_to: None,
            frame_type: FrameType::JoinRoom,
            payload: serde_json::json!({"room_id": "lobby"}),
        };
        socket
            .send(ClientMessage::Text(
                join.to_line().unwrap().trim_end().to_owned().into(),
            ))
            .await
            .unwrap();
        while next_text_frame(&mut socket).await.reply_to.as_deref() != Some("join-before-cancel") {
        }
        assert!(observed_state.broker.is_agent_in_room(agent_id, "lobby"));

        let subscribe = Frame {
            id: Some("blocked-subscribe".into()),
            reply_to: None,
            frame_type: FrameType::Subscribe,
            payload: serde_json::json!({
                "room_id": "lobby",
                "webhook_url": "https://example.com/cowchat",
                "secret": "test-only-secret"
            }),
        };
        socket
            .send(ClientMessage::Text(
                subscribe.to_line().unwrap().trim_end().to_owned().into(),
            ))
            .await
            .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            validation_started.notified(),
        )
        .await
        .expect("subscribe command should enter the controlled validation wait");

        socket.send(ClientMessage::Close(None)).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while observed_state.broker.agents.contains_key(agent_id)
                || observed_state.broker.is_agent_in_room(agent_id, "lobby")
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("transport close must cancel the command and finish agent cleanup");

        server.abort();
    }

    #[tokio::test]
    async fn websocket_connection_guard_aborts_bridges_and_signals_cleanup() {
        let transport_disconnect = Arc::new(tokio::sync::Notify::new());
        let input = tokio::spawn(std::future::pending::<()>());
        let output = tokio::spawn(std::future::pending::<()>());
        let guard = WebSocketConnectionGuard {
            transport_disconnect: transport_disconnect.clone(),
            input_abort: input.abort_handle(),
            output_abort: output.abort_handle(),
            armed: true,
        };

        drop(guard);

        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            transport_disconnect.notified(),
        )
        .await
        .expect("dropping the outer WebSocket handler must wake connection cleanup");
        assert!(input.await.unwrap_err().is_cancelled());
        assert!(output.await.unwrap_err().is_cancelled());
    }

    async fn next_text_frame<S>(socket: &mut S) -> Frame
    where
        S: futures::Stream<Item = Result<ClientMessage, tokio_tungstenite::tungstenite::Error>>
            + Unpin,
    {
        let message = tokio::time::timeout(std::time::Duration::from_secs(1), socket.next())
            .await
            .expect("server should send a WebSocket frame")
            .expect("server closed before sending the expected frame")
            .expect("WebSocket read should succeed");
        let ClientMessage::Text(text) = message else {
            panic!("expected WebSocket text frame, got {message:?}");
        };
        Frame::from_line(&text).unwrap()
    }

    #[test]
    fn browser_origin_must_be_explicitly_allowed() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        assert!(!origin_allowed(
            &headers,
            &["https://cowchat.example".into()]
        ));
        assert!(origin_allowed(&headers, &["https://evil.example".into()]));
    }

    #[test]
    fn peer_ip_is_default_and_xff_requires_an_explicit_trusted_proxy() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("203.0.113.10, 198.51.100.20"),
        );
        let direct_a: SocketAddr = "192.0.2.10:1111".parse().unwrap();
        let direct_b: SocketAddr = "192.0.2.11:2222".parse().unwrap();
        assert_eq!(signup_bucket(direct_a, &headers, &[]), "192.0.2.10");
        assert_eq!(signup_bucket(direct_b, &headers, &[]), "192.0.2.11");
        assert_eq!(
            signup_bucket(direct_a, &headers, &["192.0.2.99".parse().unwrap()]),
            "192.0.2.10",
            "an untrusted direct peer must not spoof XFF even when proxies are configured"
        );
        assert_eq!(
            signup_bucket(direct_a, &headers, &[direct_a.ip()]),
            "198.51.100.20"
        );
    }

    #[tokio::test]
    async fn signup_requires_explicit_mode_and_admin_secret_when_configured() {
        let peer = ConnectInfo("127.0.0.1:1234".parse().unwrap());
        let disabled = create_api_key(State(test_state()), peer, HeaderMap::new(), None)
            .await
            .into_response();
        assert_eq!(disabled.status(), StatusCode::FORBIDDEN);

        let mut enabled = test_state();
        enabled.signup_enabled = true;
        enabled.admin_secret = Some("admin-secret".into());
        let missing = create_api_key(State(enabled.clone()), peer, HeaderMap::new(), None)
            .await
            .into_response();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HeaderName::from_static(ADMIN_HEADER),
            HeaderValue::from_static("admin-secret"),
        );
        let created = create_api_key(State(enabled), peer, headers, None)
            .await
            .into_response();
        assert_eq!(created.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn signup_without_admin_secret_is_open_and_rate_limited() {
        let peer = ConnectInfo("127.0.0.1:1234".parse().unwrap());
        let mut open = test_state();
        open.signup_enabled = true;
        open.admin_secret = None;

        let created = create_api_key(State(open.clone()), peer, HeaderMap::new(), None)
            .await
            .into_response();
        assert_eq!(created.status(), StatusCode::CREATED);

        // A stray admin header on an open server must not break signup.
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HeaderName::from_static(ADMIN_HEADER),
            HeaderValue::from_static("ignored"),
        );
        let with_header = create_api_key(State(open.clone()), peer, headers, None)
            .await
            .into_response();
        assert_eq!(with_header.status(), StatusCode::CREATED);

        // The per-IP mint throttle is the only brake in open mode.
        let mut throttled_status = StatusCode::CREATED;
        for _ in 0..64 {
            let response = create_api_key(State(open.clone()), peer, HeaderMap::new(), None)
                .await
                .into_response();
            throttled_status = response.status();
            if throttled_status == StatusCode::TOO_MANY_REQUESTS {
                break;
            }
        }
        assert_eq!(throttled_status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn global_agent_rest_endpoint_requires_a_key() {
        let response = api_list_agents(State(test_state()), HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
