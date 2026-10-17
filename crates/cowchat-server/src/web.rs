use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, DefaultBodyLimit, Path, Query, State,
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
use crate::session_auth::SessionAuth;
use crate::store::Store;
use crate::tasks::TaskManager;
use crate::voting::VoteManager;
use cowchat_core::SessionChallengeRequest;

const ADMIN_HEADER: &str = "x-cowchat-admin";
const API_KEY_HEADER: &str = "x-cowchat-key";
const SHA256_HEADER: &str = "x-cowchat-sha256";
const WEBSOCKET_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// Hard cap on a single blob attachment.
pub(crate) const MAX_BLOB_BYTES: usize = 25 * 1024 * 1024;
/// Max Unicode scalar values kept from an uploaded blob's `name`.
const MAX_BLOB_NAME_CHARS: usize = 128;

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
    pub session_auth: Option<Arc<SessionAuth>>,
}

pub fn router(state: AppState) -> Router {
    let allowed_origins = state.allowed_origins.clone();
    let hosted = state.broker.is_hosted();
    let router = Router::new()
        .route("/ws", get(ws_handler))
        .route(
            "/auth/session/challenge",
            post(issue_session_challenge).layer(DefaultBodyLimit::max(4 * 1024)),
        )
        .route("/api/keys", post(create_api_key))
        .route("/api/invites/redeem", post(redeem_invite))
        .route("/api/status", get(api_status))
        .route("/api/rooms", get(api_list_rooms))
        .route("/api/agents", get(api_list_agents))
        .route("/api/rooms/{room_id}/history", get(api_room_history))
        .route(
            "/api/rooms/{room_id}/artifact",
            get(artifact_get)
                .put(artifact_publish)
                .patch(artifact_patch)
                .delete(artifact_unpublish)
                .layer(DefaultBodyLimit::max(
                    crate::artifact::MAX_ARTIFACT_BYTES + 128 * 1024,
                )),
        )
        .route(
            "/api/rooms/{room_id}/artifact/rotate",
            post(artifact_rotate).layer(DefaultBodyLimit::max(4096)),
        )
        .route("/a/{public_id}", get(artifact_shell))
        .route("/a/{public_id}/snapshot", get(artifact_snapshot))
        .route("/a/{public_id}/events", get(artifact_events))
        .route(
            "/api/rooms/{room_id}/blobs",
            // Let bodies just over the cap through the framework limit so the
            // handler can answer the documented JSON 413.
            post(upload_blob).layer(DefaultBodyLimit::max(MAX_BLOB_BYTES + 1024)),
        )
        .route("/api/blobs/{blob_id}", get(download_blob))
        .fallback(static_handler)
        .with_state(state)
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| async move {
                // Initial hosted slice exposes the authenticated frame protocol.
                // Refuse all REST storage/administration routes, including invite
                // redemption and blobs that bypass the frame dispatcher.
                if hosted && request.uri().path().starts_with("/api/") {
                    return (
                        StatusCode::NOT_IMPLEMENTED,
                        "This HTTP operation is not enabled in hosted mode",
                    )
                        .into_response();
                }
                let artifact = request.uri().path().starts_with("/a/")
                    || (request.uri().path().starts_with("/api/rooms/")
                        && request.uri().path().contains("/artifact"));
                let response = next.run(request).await;
                if artifact {
                    artifact_headers(response)
                } else {
                    response
                }
            },
        ));
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
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                ])
                .allow_headers([
                    header::CONTENT_TYPE,
                    header::HeaderName::from_static(ADMIN_HEADER),
                    header::HeaderName::from_static(API_KEY_HEADER),
                ]),
        )
    }
}

async fn issue_session_challenge(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<SessionChallengeRequest>,
) -> axum::response::Response {
    let Some(session_auth) = state.session_auth.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !origin_allowed(&headers, &state.allowed_origins) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let client_ip = signup_bucket(peer, &headers, &state.trusted_proxy_ips);
    if !state.rate_limiter.try_register_session(&client_ip) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    match session_auth.issue(request, origin) {
        Ok(challenge) => Json(challenge).into_response(),
        Err(_) => json_error(StatusCode::BAD_REQUEST, "Invalid session request"),
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
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    ws.on_upgrade(move |socket| handle_ws_connection(socket, state, origin))
        .into_response()
}

async fn handle_ws_connection(ws: WebSocket, state: AppState, origin: String) {
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
    let session_auth = state.session_auth;
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
            session_auth,
            Some(origin),
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

/// Redeem a room-invite token for a freshly minted API key plus a grant to
/// the invite's room. Unauthenticated by design — the token IS the
/// authorization — and independent of `--enable-http-signup` (an invite is an
/// explicit grant, not open signup), but throttled by the same per-IP signup
/// rate limiter. Unknown, revoked, and used-up tokens all return the same
/// generic 404 so the endpoint is not an invite-state oracle.
async fn redeem_invite(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    let client_ip = signup_bucket(peer, &headers, &state.trusted_proxy_ips);
    if !state.rate_limiter.try_register_signup(&client_ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": "redemption rate limit exceeded for your IP; try again later"
            })),
        );
    }

    let invalid = || {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "invalid invite token"})),
        )
    };
    let Some(token) = body
        .as_ref()
        .and_then(|b| b.get("token"))
        .and_then(|v| v.as_str())
        .filter(|t| !t.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "token is required"})),
        );
    };

    let token_hash = crate::auth::hash_invite_token(token);
    let room_id = match state.store.redeem_invite(&token_hash) {
        Ok(Some(room_id)) => room_id,
        Ok(None) => return invalid(),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        }
    };
    // Room-destroy cleanup deletes invites, so this only misses on a narrow
    // destroy race; fail with the same generic body.
    let Some(room) = state.store.get_room(&room_id).ok().flatten() else {
        return invalid();
    };

    let api_key = crate::auth::generate_key();
    let label = format!("invite:{}", room.name);
    if let Err(e) = state.store.create_api_key(&api_key, Some(&label)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        );
    }
    // The grant must be durable before the key is handed out.
    if let Err(e) = state.store.add_room_grant(&api_key, &room_id) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        );
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "api_key": api_key,
            "room_id": room.room_id,
            "room_name": room.name,
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

async fn api_list_rooms(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    // Without a key: only public rooms. With a valid key: also private rooms
    // owned by, or invite-granted to, that key.
    let key = headers
        .get(API_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|key| *key == state.api_key || state.store.validate_api_key(key).unwrap_or(false));
    let rooms = state
        .store
        .list_rooms_for_key(key, None)
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

// --- Explicitly published room artifacts ---

fn artifact_headers(mut response: axum::response::Response) -> axum::response::Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-robots-tag",
        HeaderValue::from_static("noindex, nofollow"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn artifact_error(error: crate::artifact::ArtifactError) -> axum::response::Response {
    use crate::artifact::ArtifactError;
    let status = match &error {
        ArtifactError::NotFound => StatusCode::NOT_FOUND,
        ArtifactError::Conflict => StatusCode::CONFLICT,
        ArtifactError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        ArtifactError::Invalid(_) => StatusCode::BAD_REQUEST,
        ArtifactError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    artifact_headers(json_error(status, &error.to_string()))
}

fn artifact_room_key(
    state: &AppState,
    headers: &HeaderMap,
    room_id: &str,
) -> Result<String, StatusCode> {
    let key = authenticated_key(state, headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let accessible = state
        .store
        .get_room(room_id)
        .ok()
        .flatten()
        .is_some_and(|room| {
            crate::connection::can_access_room(&room, &key, state.no_auth, &state.store)
        });
    if !accessible {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(key)
}

async fn artifact_get(
    State(state): State<AppState>,
    Path(room): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    let key = match artifact_room_key(&state, &headers, &room) {
        Ok(key) => key,
        Err(status) => return artifact_headers(status.into_response()),
    };
    match state
        .store
        .get_authorized_artifact(&room, &key, state.no_auth)
    {
        Ok(Some(artifact)) => artifact_headers(Json(artifact).into_response()),
        Ok(None) => artifact_error(crate::artifact::ArtifactError::NotFound),
        Err(error) => artifact_error(error),
    }
}

async fn artifact_publish(
    State(state): State<AppState>,
    Path(room): Path<String>,
    headers: HeaderMap,
    Json(request): Json<crate::artifact::PublishArtifact>,
) -> axum::response::Response {
    let key = match artifact_room_key(&state, &headers, &room) {
        Ok(key) => key,
        Err(status) => return artifact_headers(status.into_response()),
    };
    match state
        .store
        .publish_artifact(&room, &key, state.no_auth, &request)
    {
        Ok(artifact) => artifact_headers(Json(artifact).into_response()),
        Err(error) => artifact_error(error),
    }
}

async fn artifact_patch(
    State(state): State<AppState>,
    Path(room): Path<String>,
    headers: HeaderMap,
    Json(request): Json<crate::artifact::PatchArtifact>,
) -> axum::response::Response {
    let key = match artifact_room_key(&state, &headers, &room) {
        Ok(key) => key,
        Err(status) => return artifact_headers(status.into_response()),
    };
    match state
        .store
        .patch_artifact(&room, &key, state.no_auth, &request)
    {
        Ok(artifact) => artifact_headers(Json(artifact).into_response()),
        Err(error) => artifact_error(error),
    }
}

async fn artifact_rotate(
    State(state): State<AppState>,
    Path(room): Path<String>,
    headers: HeaderMap,
    Json(request): Json<crate::artifact::RotateArtifact>,
) -> axum::response::Response {
    let key = match artifact_room_key(&state, &headers, &room) {
        Ok(key) => key,
        Err(status) => return artifact_headers(status.into_response()),
    };
    match state
        .store
        .rotate_artifact(&room, &key, state.no_auth, &request.request_id)
    {
        Ok(artifact) => artifact_headers(Json(artifact).into_response()),
        Err(error) => artifact_error(error),
    }
}

async fn artifact_unpublish(
    State(state): State<AppState>,
    Path(room): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    let key = match artifact_room_key(&state, &headers, &room) {
        Ok(key) => key,
        Err(status) => return artifact_headers(status.into_response()),
    };
    match state.store.unpublish_artifact(&room, &key, state.no_auth) {
        Ok(()) => artifact_headers(StatusCode::NO_CONTENT.into_response()),
        Err(error) => artifact_error(error),
    }
}

fn public_snapshot(artifact: crate::artifact::Artifact) -> serde_json::Value {
    serde_json::json!({"title":artifact.title,"view":artifact.view,"data":artifact.data,"revision":artifact.revision,"updated_at":artifact.updated_at})
}

async fn artifact_snapshot(
    State(state): State<AppState>,
    Path(public_id): Path<String>,
) -> axum::response::Response {
    match state.store.get_public_artifact(&public_id) {
        Ok(Some(artifact)) => artifact_headers(Json(public_snapshot(artifact)).into_response()),
        Ok(None) => artifact_error(crate::artifact::ArtifactError::NotFound),
        Err(error) => artifact_error(error),
    }
}

async fn artifact_shell(
    State(state): State<AppState>,
    Path(public_id): Path<String>,
) -> axum::response::Response {
    match state.store.get_public_artifact(&public_id) {
        Ok(Some(_)) => match WebAssets::get("artifact.html") {
            Some(asset) => {
                let mut response = (
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    asset.data.into_owned(),
                )
                    .into_response();
                response.headers_mut().insert("content-security-policy",HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; font-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"));
                artifact_headers(response)
            }
            None => artifact_headers(StatusCode::SERVICE_UNAVAILABLE.into_response()),
        },
        Ok(None) => artifact_error(crate::artifact::ArtifactError::NotFound),
        Err(error) => artifact_error(error),
    }
}

async fn artifact_events(
    State(state): State<AppState>,
    Path(public_id): Path<String>,
) -> axum::response::Response {
    use axum::response::sse::{Event, KeepAlive, Sse};
    // Subscribe before the first read so a commit between connect and snapshot cannot be lost.
    let changes = state.store.artifact_changes.subscribe();
    let artifact = match state.store.get_public_artifact(&public_id) {
        Ok(Some(artifact)) => artifact,
        Ok(None) => return artifact_error(crate::artifact::ArtifactError::NotFound),
        Err(error) => return artifact_error(error),
    };
    let initial = Some(artifact.revision);
    let stream = futures::stream::unfold(
        (state.store, public_id, changes, initial, 0_u64, false),
        |(store, id, mut changes, mut pending, mut last, closed)| async move {
            if closed {
                return None;
            }
            loop {
                if let Some(revision) = pending.take() {
                    last = revision;
                    let event = Event::default()
                        .event("revision")
                        .id(revision.to_string())
                        .data(serde_json::json!({"revision":revision}).to_string());
                    return Some((
                        Ok::<_, std::convert::Infallible>(event),
                        (store, id, changes, pending, last, false),
                    ));
                }
                if changes.changed().await.is_err() {
                    return None;
                }
                match store.get_public_artifact(&id) {
                    Ok(Some(artifact)) if artifact.revision != last => {
                        pending = Some(artifact.revision)
                    }
                    Ok(Some(_)) => {}
                    Err(_) => return None,
                    Ok(None) => {
                        return Some((
                            Ok(Event::default().event("revoked").data("{}")),
                            (store, id, changes, None, last, true),
                        ))
                    }
                }
            }
        },
    );
    let mut response = artifact_headers(
        Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response(),
    );
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

// --- Blob attachments ---

/// The API key presented in `x-cowchat-key`, if it's valid on this server.
fn authenticated_key(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let key = headers.get(API_KEY_HEADER)?.to_str().ok()?;
    if key.is_empty() {
        return None;
    }
    (key == state.api_key || state.store.validate_api_key(key).unwrap_or(false))
        .then(|| key.to_string())
}

/// Reduce an uploaded filename to a safe basename: last path component only,
/// control characters and quotes stripped, trimmed, capped at
/// [`MAX_BLOB_NAME_CHARS`]. None = nothing usable left.
fn sanitize_blob_name(raw: &str) -> Option<String> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && *c != '"')
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_BLOB_NAME_CHARS).collect())
}

fn json_error(status: StatusCode, message: &str) -> axum::response::Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// Generic 404 for both "room/blob doesn't exist" and "you can't access it" —
/// same no-oracle stance as invite redemption.
fn blob_not_found() -> axum::response::Response {
    json_error(StatusCode::NOT_FOUND, "blob not found")
}

fn room_not_found() -> axum::response::Response {
    json_error(StatusCode::NOT_FOUND, "room not found")
}

#[derive(serde::Deserialize)]
struct UploadBlobParams {
    name: Option<String>,
}

async fn upload_blob(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Query(params): Query<UploadBlobParams>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Some(key) = authenticated_key(&state, &headers) else {
        return json_error(StatusCode::UNAUTHORIZED, "invalid API key");
    };
    // Absent rooms and inaccessible private rooms answer identically.
    let accessible = state
        .store
        .get_room(&room_id)
        .ok()
        .flatten()
        .is_some_and(|room| {
            crate::connection::can_access_room(&room, &key, state.no_auth, &state.store)
        });
    if !accessible {
        return room_not_found();
    }
    let Some(name) = params.name.as_deref().and_then(sanitize_blob_name) else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "a non-empty ?name=<filename> query parameter is required",
        );
    };
    if body.len() > MAX_BLOB_BYTES {
        return json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "blob exceeds the 25 MiB attachment limit",
        );
    }
    if !state.rate_limiter.try_register_upload(&key) {
        return json_error(
            StatusCode::TOO_MANY_REQUESTS,
            "upload rate limit exceeded for this API key; try again later",
        );
    }

    match state.store.add_blob(&room_id, &name, &body) {
        Ok(blob) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "blob_id": blob.blob_id,
                "room_id": blob.room_id,
                "name": blob.name,
                "sha256": blob.sha256,
                "size": blob.size,
            })),
        )
            .into_response(),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn download_blob(
    State(state): State<AppState>,
    Path(blob_id): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    let Some(key) = authenticated_key(&state, &headers) else {
        return json_error(StatusCode::UNAUTHORIZED, "invalid API key");
    };
    let Ok(Some(blob)) = state.store.get_blob(&blob_id) else {
        return blob_not_found();
    };
    // Access is judged against the blob's room; unknown ids, destroyed rooms,
    // and inaccessible rooms all answer identically.
    let accessible = state
        .store
        .get_room(&blob.room_id)
        .ok()
        .flatten()
        .is_some_and(|room| {
            crate::connection::can_access_room(&room, &key, state.no_auth, &state.store)
        });
    if !accessible {
        return blob_not_found();
    }
    let Ok(bytes) = tokio::fs::read(state.store.blob_file_path(&blob.blob_id)).await else {
        log::warn!("blob {} has a row but no readable file", blob.blob_id);
        return blob_not_found();
    };

    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", blob.name))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    let sha_header =
        HeaderValue::from_str(&blob.sha256).expect("sha256 hex is always a valid header value");
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            ),
            (header::CONTENT_DISPOSITION, disposition),
            (header::HeaderName::from_static(SHA256_HEADER), sha_header),
        ],
        bytes,
    )
        .into_response()
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
    use cowchat_core::{ErrorCode, ErrorPayload, Frame, FrameType, RegisterPayload, SessionProof};
    use dashmap::DashMap;
    use k256::ecdsa::{signature::hazmat::PrehashSigner, Signature, SigningKey};
    use rand::rngs::OsRng;
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{client::IntoClientRequest, Message as ClientMessage},
    };

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
            rate_limiter: Arc::new(RateLimiter::new(None)),
            no_auth: false,
            api_key: "master".into(),
            reconnect_mgr: Arc::new(ReconnectManager::new()),
            task_mgr: Arc::new(TaskManager::new(store.clone())),
            webhook_mgr: Arc::new(crate::webhooks::WebhookManager::new(store, true)),
            signup_enabled: false,
            admin_secret: None,
            allowed_origins: vec![],
            trusted_proxy_ips: vec![],
            session_auth: Some(Arc::new(
                SessionAuth::new([0xab; 32], "wss://chat.example/ws".into()).unwrap(),
            )),
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
    async fn artifact_http_auth_capability_and_live_isolation() {
        let state = test_state();
        for room in ["target", "other"] {
            state
                .store
                .create_room_with_visibility(
                    room,
                    room,
                    None,
                    None,
                    Some("author"),
                    "private",
                    Some("master"),
                    false,
                )
                .unwrap();
        }
        state.store.create_api_key("outsider", None).unwrap();
        let observed = state.store.clone();
        let (server, addr) = start_test_web_server(state).await;
        let client = reqwest::Client::new();
        let base = format!("http://{addr}");
        let request = serde_json::json!({"title":"Board","view":"root = Stack([])","data":{"items":{"a":1}},"request_id":uuid::Uuid::new_v4()});
        let url = format!("{base}/api/rooms/target/artifact");
        assert_eq!(
            client
                .put(&url)
                .json(&request)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        for room in ["target", "missing"] {
            let response = client
                .put(format!("{base}/api/rooms/{room}/artifact"))
                .header(API_KEY_HEADER, "outsider")
                .json(&request)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        let response = client
            .put(&url)
            .header(API_KEY_HEADER, "master")
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let first: serde_json::Value = response.json().await.unwrap();
        let id = first["public_id"].as_str().unwrap();
        let snapshot = format!("{base}/a/{id}/snapshot");
        let public: serde_json::Value = client
            .get(&snapshot)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(public["data"]["items"]["a"], 1);
        assert!(public.get("room_id").is_none());
        assert!(public.get("public_id").is_none());
        assert_eq!(
            client
                .patch(&snapshot)
                .json(&request)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            client
                .get(format!("{base}/a/{}/snapshot", uuid::Uuid::new_v4()))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        let mut events = client
            .get(format!("{base}/a/{id}/events"))
            .send()
            .await
            .unwrap();
        assert_eq!(events.headers()["x-accel-buffering"], "no");
        let first_event = events.chunk().await.unwrap().unwrap();
        assert!(String::from_utf8_lossy(&first_event).contains("\"revision\":1"));
        let other = crate::artifact::PublishArtifact {
            title: "Other".into(),
            view: "root = Stack([])".into(),
            data: None,
            expected_revision: None,
            request_id: uuid::Uuid::new_v4().to_string(),
        };
        observed
            .publish_artifact("other", "master", false, &other)
            .unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(80), events.chunk())
                .await
                .is_err()
        );
        let patch =
            serde_json::json!({"patch":{"items":{"b":2}},"request_id":uuid::Uuid::new_v4()});
        let changed: serde_json::Value = client
            .patch(&url)
            .header(API_KEY_HEADER, "master")
            .json(&patch)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(changed["revision"], 2);
        assert_eq!(changed["data"]["items"], serde_json::json!({"a":1,"b":2}));
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.chunk())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&event).contains("\"revision\":2"));
        let mut reconnect = client
            .get(format!("{base}/a/{id}/events"))
            .header("last-event-id", "1")
            .send()
            .await
            .unwrap();
        assert!(
            String::from_utf8_lossy(&reconnect.chunk().await.unwrap().unwrap())
                .contains("\"revision\":2")
        );
        let rotated: serde_json::Value = client
            .post(format!("{url}/rotate"))
            .header(API_KEY_HEADER, "master")
            .json(&serde_json::json!({"request_id":uuid::Uuid::new_v4()}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.chunk())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&event).contains("revoked"));
        assert_eq!(
            client.get(&snapshot).send().await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            client
                .get(format!("{base}/a/{id}/events"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        let id = rotated["public_id"].as_str().unwrap();
        observed
            .destroy_room_authorized("target", "author", "master", false)
            .unwrap();
        assert_eq!(
            client
                .get(format!("{base}/a/{id}/snapshot"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        server.abort();
    }

    #[tokio::test]
    async fn artifact_http_rejects_invalid_bounds_and_guards_view_replacement() {
        let state = test_state();
        state
            .store
            .create_room_with_visibility(
                "target",
                "Target",
                None,
                None,
                Some("author"),
                "private",
                Some("master"),
                false,
            )
            .unwrap();
        state.store.create_api_key("guest", None).unwrap();
        state.store.add_room_grant("guest", "target").unwrap();
        let (server, addr) = start_test_web_server(state).await;
        let client = reqwest::Client::new();
        let url = format!("http://{addr}/api/rooms/target/artifact");
        let make = || serde_json::json!({"title":"Board","view":"root = Stack([])","request_id":uuid::Uuid::new_v4()});
        let result = client
            .put(&url)
            .header(API_KEY_HEADER, "guest")
            .json(&make())
            .send()
            .await
            .unwrap();
        assert_eq!(result.status(), StatusCode::OK);
        let result = client
            .put(&url)
            .header(API_KEY_HEADER, "guest")
            .json(&make())
            .send()
            .await
            .unwrap();
        assert_eq!(result.status(), StatusCode::CONFLICT);
        let mut replace = make();
        replace["expected_revision"] = 1.into();
        assert_eq!(
            client
                .put(&url)
                .header(API_KEY_HEADER, "master")
                .json(&replace)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        for patch in [
            serde_json::json!([]),
            serde_json::json!({"x":vec![0;20_001]}),
            serde_json::json!({"deploy":{"steps":["build","test"]}}),
            serde_json::json!({"rows":{"constructor":{"name":"reserved"}}}),
            serde_json::json!({"row":{"type":"element"}}),
            serde_json::json!({"row":{"k":"Comp"}}),
            serde_json::json!({"row":{"__reactive":true}}),
        ] {
            let response = client
                .patch(&url)
                .header(API_KEY_HEADER, "guest")
                .json(&serde_json::json!({"patch":patch,"request_id":uuid::Uuid::new_v4()}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let current: serde_json::Value = client
                .get(&url)
                .header(API_KEY_HEADER, "guest")
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(current["revision"], 1);
            assert_eq!(current["data"], serde_json::json!({}));
        }
        let mut invalid = make();
        invalid["expected_revision"] = 1.into();
        invalid["data"] = serde_json::Value::Null;
        assert_eq!(
            client
                .put(&url)
                .header(API_KEY_HEADER, "guest")
                .json(&invalid)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        invalid["data"] = serde_json::json!({});
        invalid["view"] = "x".repeat(crate::artifact::MAX_VIEW_BYTES + 1).into();
        assert_eq!(
            client
                .put(&url)
                .header(API_KEY_HEADER, "guest")
                .json(&invalid)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        invalid["view"] = "x".repeat(crate::artifact::MAX_ARTIFACT_BYTES * 2).into();
        let response = client
            .put(&url)
            .header(API_KEY_HEADER, "guest")
            .json(&invalid)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            client
                .delete(&url)
                .header(API_KEY_HEADER, "guest")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            client
                .get(&url)
                .header(API_KEY_HEADER, "guest")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        server.abort();
    }

    async fn issue_test_session(addr: SocketAddr) -> (SessionProof, String) {
        let wallet = SigningKey::random(&mut OsRng);
        let ephemeral = SigningKey::random(&mut OsRng);
        let principal_address = crate::session_auth::address_for_key(wallet.verifying_key());
        let request = reqwest::Client::new()
            .post(format!("http://{addr}/auth/session/challenge"))
            .header(header::ORIGIN, "https://dashboard.example")
            .json(&serde_json::json!({
                "principal_address": principal_address,
                "session_public_key": format!(
                    "0x{}",
                    hex::encode(ephemeral.verifying_key().to_encoded_point(true).as_bytes())
                ),
                "audience": cowchat_core::SESSION_BROWSER_AUDIENCE,
                "endpoint": "wss://chat.example/ws",
            }));
        let challenge = request
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<cowchat_core::SessionChallengeResponse>()
            .await
            .unwrap();
        let digest = cowchat_core::session_challenge_digest(&challenge);
        let (principal_signature, recovery) = wallet.sign_prehash_recoverable(&digest).unwrap();
        let mut principal_signature = principal_signature.to_bytes().to_vec();
        principal_signature.push(recovery.to_byte());
        let possession: Signature = ephemeral
            .sign_prehash(&cowchat_core::session_possession_digest(&digest))
            .unwrap();
        (
            SessionProof {
                nonce: challenge.nonce,
                principal_signature: format!("0x{}", hex::encode(principal_signature)),
                possession_signature: format!("0x{}", hex::encode(possession.to_bytes())),
            },
            principal_address,
        )
    }

    async fn register_test_session(addr: SocketAddr, proof: SessionProof, key: &str) -> Frame {
        let mut request = format!("ws://{addr}/ws").into_client_request().unwrap();
        request
            .headers_mut()
            .insert(header::ORIGIN, "https://dashboard.example".parse().unwrap());
        let (mut socket, _) = connect_async(request).await.unwrap();
        socket
            .send(ClientMessage::Text(
                Frame {
                    id: Some("session-register".into()),
                    reply_to: None,
                    frame_type: FrameType::Register,
                    payload: serde_json::to_value(RegisterPayload {
                        key: key.into(),
                        session: Some(proof),
                        agent_id: Some("attacker-selected".into()),
                        name: "Wallet user".into(),
                        capabilities: vec![],
                        reconnect: true,
                        protocol_version: Some(cowchat_core::PROTOCOL_VERSION),
                    })
                    .unwrap(),
                }
                .to_line()
                .unwrap()
                .trim_end()
                .into(),
            ))
            .await
            .unwrap();
        let reply = socket.next().await.unwrap().unwrap();
        let reply = Frame::from_line(reply.to_text().unwrap()).unwrap();
        socket.close(None).await.unwrap();
        reply
    }

    #[tokio::test]
    async fn member_session_rejects_a_transport_api_key() {
        let mut state = test_state();
        state.allowed_origins = vec!["https://dashboard.example".into()];
        let observed = state.clone();
        let (server, addr) = start_test_web_server(state).await;
        let (proof, principal_address) = issue_test_session(addr).await;

        let reply = register_test_session(addr, proof, "master").await;
        assert_eq!(reply.frame_type, FrameType::Error);
        assert_eq!(reply.payload["code"], "invalid_payload");
        assert!(!observed
            .broker
            .agents
            .contains_key(&format!("member:{principal_address}")));
        server.abort();
    }

    #[tokio::test]
    async fn browser_challenge_registers_only_the_server_derived_wallet_principal() {
        let mut state = test_state();
        state.allowed_origins = vec!["https://dashboard.example".into()];
        let observed = state.clone();
        let (server, addr) = start_test_web_server(state).await;

        let wallet = SigningKey::random(&mut OsRng);
        let ephemeral = SigningKey::random(&mut OsRng);
        let principal_address = crate::session_auth::address_for_key(wallet.verifying_key());
        let challenge = reqwest::Client::new()
            .post(format!("http://{addr}/auth/session/challenge"))
            .header(header::ORIGIN, "https://dashboard.example")
            .json(&serde_json::json!({
                "principal_address": principal_address,
                "session_public_key": format!(
                    "0x{}",
                    hex::encode(ephemeral.verifying_key().to_encoded_point(true).as_bytes())
                ),
                "audience": cowchat_core::SESSION_BROWSER_AUDIENCE,
                "endpoint": "wss://chat.example/ws",
            }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<cowchat_core::SessionChallengeResponse>()
            .await
            .unwrap();
        let digest = cowchat_core::session_challenge_digest(&challenge);
        let (principal_signature, recovery) = wallet.sign_prehash_recoverable(&digest).unwrap();
        let mut principal_signature = principal_signature.to_bytes().to_vec();
        principal_signature.push(recovery.to_byte());
        let possession: Signature = ephemeral
            .sign_prehash(&cowchat_core::session_possession_digest(&digest))
            .unwrap();

        let mut request = format!("ws://{addr}/ws").into_client_request().unwrap();
        request
            .headers_mut()
            .insert(header::ORIGIN, "https://dashboard.example".parse().unwrap());
        let (mut socket, _) = connect_async(request).await.unwrap();
        socket
            .send(ClientMessage::Text(
                Frame {
                    id: Some("wallet-register".into()),
                    reply_to: None,
                    frame_type: FrameType::Register,
                    payload: serde_json::to_value(RegisterPayload {
                        key: String::new(),
                        session: Some(SessionProof {
                            nonce: challenge.nonce,
                            principal_signature: format!("0x{}", hex::encode(principal_signature)),
                            possession_signature: format!(
                                "0x{}",
                                hex::encode(possession.to_bytes())
                            ),
                        }),
                        // This must be ignored rather than becoming the identity.
                        agent_id: Some("attacker-selected".into()),
                        name: "Wallet user".into(),
                        capabilities: vec![],
                        reconnect: true,
                        protocol_version: Some(cowchat_core::PROTOCOL_VERSION),
                    })
                    .unwrap(),
                }
                .to_line()
                .unwrap()
                .trim_end()
                .into(),
            ))
            .await
            .unwrap();
        let reply = socket.next().await.unwrap().unwrap();
        let reply = Frame::from_line(reply.to_text().unwrap()).unwrap();
        let expected_id = format!("member:{principal_address}");
        assert_eq!(reply.payload["agent_id"], expected_id);
        let connection = observed.broker.agents.get(&expected_id).unwrap();
        assert!(connection.api_key.is_empty());
        assert_eq!(connection.member_address, Some(principal_address));
        drop(connection);
        socket.close(None).await.unwrap();
        server.abort();
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
                session: None,
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
                session: None,
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
                session: None,
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

    async fn response_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// Private room owned by `owner-key`, with `granted-key` invited in and
    /// `other-key` valid but ungranted.
    fn seed_blob_room(state: &AppState) {
        for key in ["owner-key", "granted-key", "other-key"] {
            state.store.create_api_key(key, None).unwrap();
        }
        state
            .store
            .create_room_with_visibility(
                "files-room",
                "files-room",
                None,
                None,
                Some("creator"),
                "private",
                Some("owner-key"),
                false,
            )
            .unwrap();
        state
            .store
            .add_room_grant("granted-key", "files-room")
            .unwrap();
    }

    fn blob_url(addr: &SocketAddr, blob_id: &str) -> String {
        format!("http://{addr}/api/blobs/{blob_id}")
    }

    fn upload_url(addr: &SocketAddr, room: &str) -> String {
        format!("http://{addr}/api/rooms/{room}/blobs")
    }

    async fn upload(
        addr: &SocketAddr,
        room: &str,
        key: &str,
        name: &str,
        bytes: Vec<u8>,
    ) -> reqwest::Response {
        reqwest::Client::new()
            .post(upload_url(addr, room))
            .query(&[("name", name)])
            .header(API_KEY_HEADER, key)
            .body(bytes)
            .send()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn blob_upload_download_roundtrip_for_owner_and_granted_keys() {
        let state = test_state();
        seed_blob_room(&state);
        let observed = state.clone();
        let (server, addr) = start_test_web_server(state).await;

        let content = b"attachment payload bytes".to_vec();
        let created = upload(
            &addr,
            "files-room",
            "owner-key",
            "notes.txt",
            content.clone(),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let body: serde_json::Value =
            serde_json::from_slice(&created.bytes().await.unwrap()).unwrap();
        assert_eq!(body["room_id"], "files-room");
        assert_eq!(body["name"], "notes.txt");
        assert_eq!(body["size"], content.len());
        assert_eq!(body["sha256"], crate::auth::sha256_hex(&content));
        let blob_id = body["blob_id"].as_str().unwrap().to_string();
        assert!(observed.store.blob_file_path(&blob_id).exists());

        for key in ["owner-key", "granted-key"] {
            let fetched = reqwest::Client::new()
                .get(blob_url(&addr, &blob_id))
                .header(API_KEY_HEADER, key)
                .send()
                .await
                .unwrap();
            assert_eq!(fetched.status(), StatusCode::OK, "key {key} must download");
            assert_eq!(
                fetched
                    .headers()
                    .get(header::CONTENT_DISPOSITION)
                    .and_then(|value| value.to_str().ok()),
                Some("attachment; filename=\"notes.txt\"")
            );
            assert_eq!(
                fetched
                    .headers()
                    .get(SHA256_HEADER)
                    .and_then(|value| value.to_str().ok()),
                Some(crate::auth::sha256_hex(&content).as_str())
            );
            assert_eq!(
                fetched.content_length(),
                Some(content.len() as u64),
                "download must carry content-length"
            );
            assert_eq!(fetched.bytes().await.unwrap().to_vec(), content);
        }

        server.abort();
    }

    #[tokio::test]
    async fn blob_endpoints_reject_missing_or_invalid_keys() {
        let state = test_state();
        seed_blob_room(&state);
        let (server, addr) = start_test_web_server(state).await;
        let http = reqwest::Client::new();

        let keyless_upload = http
            .post(upload_url(&addr, "files-room"))
            .query(&[("name", "a.txt")])
            .body(b"x".to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(keyless_upload.status(), StatusCode::UNAUTHORIZED);
        let bad_key_upload = upload(&addr, "files-room", "not-a-key", "a.txt", b"x".to_vec()).await;
        assert_eq!(bad_key_upload.status(), StatusCode::UNAUTHORIZED);

        let keyless_download = http.get(blob_url(&addr, "whatever")).send().await.unwrap();
        assert_eq!(keyless_download.status(), StatusCode::UNAUTHORIZED);
        let bad_key_download = http
            .get(blob_url(&addr, "whatever"))
            .header(API_KEY_HEADER, "not-a-key")
            .send()
            .await
            .unwrap();
        assert_eq!(bad_key_download.status(), StatusCode::UNAUTHORIZED);

        server.abort();
    }

    #[tokio::test]
    async fn blob_access_denial_is_indistinguishable_from_absence() {
        let state = test_state();
        seed_blob_room(&state);
        let (server, addr) = start_test_web_server(state).await;
        let http = reqwest::Client::new();

        let created = upload(
            &addr,
            "files-room",
            "owner-key",
            "secret.txt",
            b"s".to_vec(),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let body: serde_json::Value =
            serde_json::from_slice(&created.bytes().await.unwrap()).unwrap();
        let blob_id = body["blob_id"].as_str().unwrap().to_string();

        // Upload: a private room you can't access answers exactly like a room
        // that doesn't exist.
        let denied = upload(&addr, "files-room", "other-key", "a.txt", b"x".to_vec()).await;
        assert_eq!(denied.status(), StatusCode::NOT_FOUND);
        let denied_body = denied.bytes().await.unwrap();
        let absent = upload(&addr, "no-such-room", "other-key", "a.txt", b"x".to_vec()).await;
        assert_eq!(absent.status(), StatusCode::NOT_FOUND);
        assert_eq!(absent.bytes().await.unwrap(), denied_body);

        // Download: an inaccessible blob answers exactly like an unknown id.
        let denied = http
            .get(blob_url(&addr, &blob_id))
            .header(API_KEY_HEADER, "other-key")
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::NOT_FOUND);
        let denied_body = denied.bytes().await.unwrap();
        let unknown = http
            .get(blob_url(&addr, "00000000-0000-0000-0000-000000000000"))
            .header(API_KEY_HEADER, "other-key")
            .send()
            .await
            .unwrap();
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        assert_eq!(unknown.bytes().await.unwrap(), denied_body);

        server.abort();
    }

    #[tokio::test]
    async fn blob_upload_over_the_cap_is_rejected_with_413() {
        let state = test_state();
        seed_blob_room(&state);
        let observed = state.clone();
        let (server, addr) = start_test_web_server(state).await;

        let oversized = vec![0u8; MAX_BLOB_BYTES + 1];
        let rejected = upload(&addr, "files-room", "owner-key", "huge.bin", oversized).await;
        assert_eq!(rejected.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body: serde_json::Value =
            serde_json::from_slice(&rejected.bytes().await.unwrap()).unwrap();
        assert!(body["error"].as_str().unwrap().contains("25 MiB"));
        // Nothing may be persisted for a rejected upload.
        let dir_empty = std::fs::read_dir(observed.store.blob_dir())
            .map(|entries| entries.count() == 0)
            .unwrap_or(true);
        assert!(dir_empty, "rejected upload must not leave a file behind");

        server.abort();
    }

    #[tokio::test]
    async fn blob_upload_sanitizes_names() {
        let state = test_state();
        seed_blob_room(&state);
        let (server, addr) = start_test_web_server(state).await;

        for (raw, stored) in [
            ("../../etc/passwd", "passwd"),
            ("a\\b\\evil.txt", "evil.txt"),
            ("with\nnewline.txt", "withnewline.txt"),
            ("  padded.txt  ", "padded.txt"),
        ] {
            let created = upload(&addr, "files-room", "owner-key", raw, b"x".to_vec()).await;
            assert_eq!(created.status(), StatusCode::CREATED, "name {raw:?}");
            let body: serde_json::Value =
                serde_json::from_slice(&created.bytes().await.unwrap()).unwrap();
            assert_eq!(body["name"], stored, "name {raw:?}");
        }

        // Over-long names are capped, not rejected.
        let long_name = "n".repeat(300);
        let created = upload(&addr, "files-room", "owner-key", &long_name, b"x".to_vec()).await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let body: serde_json::Value =
            serde_json::from_slice(&created.bytes().await.unwrap()).unwrap();
        assert_eq!(body["name"].as_str().unwrap().chars().count(), 128);

        // Nothing usable left, or no name at all: 400.
        for unusable in ["", "///", "   "] {
            let rejected = upload(&addr, "files-room", "owner-key", unusable, b"x".to_vec()).await;
            assert_eq!(
                rejected.status(),
                StatusCode::BAD_REQUEST,
                "name {unusable:?}"
            );
        }
        let nameless = reqwest::Client::new()
            .post(upload_url(&addr, "files-room"))
            .header(API_KEY_HEADER, "owner-key")
            .body(b"x".to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(nameless.status(), StatusCode::BAD_REQUEST);

        server.abort();
    }

    #[tokio::test]
    async fn blob_uploads_are_rate_limited_per_key() {
        let state = test_state();
        seed_blob_room(&state);
        let (server, addr) = start_test_web_server(state).await;

        for i in 0..crate::rate_limit::UPLOAD_MAX_PER_WINDOW {
            let created = upload(&addr, "files-room", "owner-key", "tiny.bin", b"x".to_vec()).await;
            assert_eq!(created.status(), StatusCode::CREATED, "upload #{i}");
        }
        let throttled = upload(&addr, "files-room", "owner-key", "tiny.bin", b"x".to_vec()).await;
        assert_eq!(throttled.status(), StatusCode::TOO_MANY_REQUESTS);
        // A different key has its own budget.
        let other = upload(
            &addr,
            "files-room",
            "granted-key",
            "tiny.bin",
            b"x".to_vec(),
        )
        .await;
        assert_eq!(other.status(), StatusCode::CREATED);

        server.abort();
    }

    #[test]
    fn blob_name_sanitizer_covers_edge_cases() {
        assert_eq!(
            sanitize_blob_name("plain.txt").as_deref(),
            Some("plain.txt")
        );
        assert_eq!(
            sanitize_blob_name("../../etc/passwd").as_deref(),
            Some("passwd")
        );
        assert_eq!(
            sanitize_blob_name("quo\"te.txt").as_deref(),
            Some("quote.txt"),
            "quotes must be stripped so Content-Disposition can't be broken"
        );
        assert_eq!(sanitize_blob_name(""), None);
        assert_eq!(sanitize_blob_name("/"), None);
        assert_eq!(sanitize_blob_name("\u{7}\u{8}"), None);
    }

    #[tokio::test]
    async fn invite_redeem_is_independent_of_signup_and_generic_on_bad_tokens() {
        let peer = ConnectInfo("127.0.0.1:4321".parse().unwrap());
        // Signup stays disabled in test_state — an invite is an explicit
        // grant, so redemption must work anyway.
        let state = test_state();
        assert!(!state.signup_enabled);

        let room = state
            .store
            .create_room_with_visibility(
                "invited-room",
                "Invited Room",
                None,
                None,
                Some("creator"),
                "private",
                Some("owner-key"),
                false,
            )
            .unwrap();
        let token = crate::auth::generate_invite_token();
        state
            .store
            .create_invite(
                &crate::auth::hash_invite_token(&token),
                &room.room_id,
                "owner-key",
                true,
            )
            .unwrap();

        // Unknown token: generic 404.
        let unknown = redeem_invite(
            State(state.clone()),
            peer,
            HeaderMap::new(),
            Some(Json(serde_json::json!({"token": "cinv_wrong"}))),
        )
        .await
        .into_response();
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        let unknown_body = response_json(unknown).await;

        // Valid token: 201 with a freshly minted key and a recorded grant.
        let created = redeem_invite(
            State(state.clone()),
            peer,
            HeaderMap::new(),
            Some(Json(serde_json::json!({"token": token}))),
        )
        .await
        .into_response();
        assert_eq!(created.status(), StatusCode::CREATED);
        let body = response_json(created).await;
        assert_eq!(body["room_id"], "invited-room");
        assert_eq!(body["room_name"], "Invited Room");
        assert_eq!(body["tier"], "free");
        let minted = body["api_key"].as_str().unwrap().to_string();
        assert!(state.store.validate_api_key(&minted).unwrap());
        assert!(state.store.key_has_grant(&minted, "invited-room"));

        // Second redemption of the single-use invite: the SAME generic 404 as
        // an unknown token — no invite-state oracle.
        let reused = redeem_invite(
            State(state.clone()),
            peer,
            HeaderMap::new(),
            Some(Json(serde_json::json!({"token": token}))),
        )
        .await
        .into_response();
        assert_eq!(reused.status(), StatusCode::NOT_FOUND);
        assert_eq!(response_json(reused).await, unknown_body);

        // The minted key sees the private room through the keyed room listing.
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HeaderName::from_static(API_KEY_HEADER),
            HeaderValue::from_str(&minted).unwrap(),
        );
        let listed = response_json(
            api_list_rooms(State(state.clone()), headers)
                .await
                .into_response(),
        )
        .await;
        let names: Vec<&str> = listed["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|room| room["name"].as_str())
            .collect();
        assert!(names.contains(&"Invited Room"));
        // Keyless listing still shows only public rooms.
        let public_only = response_json(
            api_list_rooms(State(state), HeaderMap::new())
                .await
                .into_response(),
        )
        .await;
        let public_names: Vec<&str> = public_only["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|room| room["name"].as_str())
            .collect();
        assert!(!public_names.contains(&"Invited Room"));
    }
}
