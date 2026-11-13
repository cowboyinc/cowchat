use crate::web::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use cowchat_core::{Frame, FrameType};
use serde::Deserialize;
use serde_json::{json, Value};

pub(crate) fn broadcast_session_state(
    broker: &crate::broker::Broker,
    store: &crate::store::Store,
    event: &crate::store::social::SessionState,
) {
    let frame = Frame::event(
        FrameType::SessionState,
        json!({"agent_id":event.agent_id,"room_id":event.room_id,"state":event.state}),
    );
    let connections: Vec<(String, String)> = broker
        .agents
        .iter()
        .map(|connection| (connection.key().clone(), connection.api_key.clone()))
        .collect();
    for (agent_id, key) in connections {
        if store.social_principal(&key).is_some_and(|principal| {
            principal.account_id == event.owner_id && principal.agent_id.is_none()
        }) {
            broker.send_to_agent(&agent_id, frame.clone());
        }
    }
}

#[derive(Clone)]
struct Provider {
    id: String,
    secret: String,
    authorize: String,
    token: String,
    user: String,
    tokeninfo: Option<String>,
}
#[derive(Clone)]
pub(crate) struct Configuration {
    origin: String,
    providers: std::collections::HashMap<String, Provider>,
}
impl Configuration {
    fn environment() -> Option<Self> {
        let origin = std::env::var("COWCHAT_SOCIAL_PUBLIC_ORIGIN").ok()?;
        let url = url::Url::parse(&origin).ok()?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return None;
        }
        let mut providers = std::collections::HashMap::new();
        for (name, authorize, token, user, tokeninfo) in [
            (
                "github",
                "https://github.com/login/oauth/authorize",
                "https://github.com/login/oauth/access_token",
                "https://api.github.com/user",
                None,
            ),
            (
                "google",
                "https://accounts.google.com/o/oauth2/v2/auth",
                "https://oauth2.googleapis.com/token",
                "https://openidconnect.googleapis.com/v1/userinfo",
                Some("https://oauth2.googleapis.com/tokeninfo"),
            ),
        ] {
            if let (Ok(id), Ok(secret)) = (
                std::env::var(format!("COWCHAT_{}_CLIENT_ID", name.to_uppercase())),
                std::env::var(format!("COWCHAT_{}_CLIENT_SECRET", name.to_uppercase())),
            ) {
                if !id.is_empty() && !secret.is_empty() {
                    providers.insert(
                        name.into(),
                        Provider {
                            id,
                            secret,
                            authorize: authorize.into(),
                            token: token.into(),
                            user: user.into(),
                            tokeninfo: tokeninfo.map(str::to_owned),
                        },
                    );
                }
            }
        }
        Some(Self {
            origin: origin.trim_end_matches('/').into(),
            providers,
        })
    }
}
fn configuration(state: &AppState) -> Option<Configuration> {
    if state.no_auth || state.api_key.is_empty() || state.broker.is_hosted() {
        None
    } else {
        state
            .store
            .social_configuration
            .get_or_init(Configuration::environment)
            .clone()
    }
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/friends/{slug}", get(friend_landing))
        .route("/social/v1/capabilities", get(capabilities))
        .route(
            "/social/v1/{*path}",
            get(action)
                .post(action)
                .patch(action)
                .put(action)
                .delete(action),
        )
        .route("/auth/social/start", post(start))
        .route("/auth/social/exchange", post(exchange))
        .route("/auth/social/{provider}/callback", get(callback))
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024))
}
fn fail(error: anyhow::Error) -> Response {
    let msg = error.to_string();
    let (status, safe) = if msg == "too many sign-in attempts" {
        (StatusCode::TOO_MANY_REQUESTS, "Too many sign-in attempts")
    } else if msg == "handle_unavailable" {
        (StatusCode::CONFLICT, "handle_unavailable")
    } else if msg == "incoming_pending" {
        (StatusCode::CONFLICT, "incoming_pending")
    } else if msg == "not found" {
        (StatusCode::NOT_FOUND, "not found")
    } else if msg == "unauthorized" {
        (StatusCode::UNAUTHORIZED, "unauthorized")
    } else if msg == "human approval required" {
        (StatusCode::FORBIDDEN, "human approval required")
    } else {
        (StatusCode::BAD_REQUEST, "Request could not be completed")
    };
    (status, Json(json!({"error":safe}))).into_response()
}
fn disabled() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error":"Social sign-in is not configured"})),
    )
        .into_response()
}
async fn capabilities(State(state): State<AppState>) -> Json<Value> {
    let mut providers = configuration(&state)
        .map(|c| c.providers.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    providers.sort();
    Json(
        json!({"providers":providers,"cowboy_login":false,"setup_version":1,"handle_availability":true,"friend_links":true,"setup_handle":true}),
    )
}
fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
async fn friend_landing(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let Some(config) = configuration(&state) else {
        return disabled();
    };
    let Ok(account) = state.store.social_friend_link_account(&slug) else {
        return fail(anyhow::anyhow!("not found"));
    };
    let name = escape_html(account["display_name"].as_str().unwrap_or_default());
    let handle = escape_html(account["handle"].as_str().unwrap_or_default());
    let slug = escape_html(&slug);
    let origin = escape_html(&config.origin);
    let native = if config.origin == "https://chat.cowchat.cowboy.inc" {
        format!("cowchat://friend/{slug}")
    } else {
        let mut endpoint = url::Url::parse(&config.origin).expect("validated public origin");
        endpoint.set_scheme("wss").expect("websocket scheme");
        endpoint.set_path("/ws");
        let encoded: String =
            url::form_urlencoded::byte_serialize(endpoint.as_str().as_bytes()).collect();
        format!("cowchat://friend/{slug}?server={encoded}")
    };
    let html = format!(
        r#"<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Connect on Cowchat</title><style>body{{margin:0;background:#faf7f2;color:#28241f;font:17px system-ui,sans-serif;display:grid;place-items:center;min-height:100vh}}main{{max-width:480px;margin:24px;padding:40px;background:#fff;border:1px solid #e6ded4;border-radius:24px;box-shadow:0 10px 40px #493d2610}}.mark{{color:#b85626;font-weight:800;letter-spacing:.08em}}h1{{font-size:32px;line-height:1.15}}p{{line-height:1.6;color:#62594f}}a{{color:#99431c}}.button{{display:inline-block;background:#b85626;color:white;padding:13px 20px;border-radius:12px;text-decoration:none;font-weight:650}}.link{{overflow-wrap:anywhere;font-size:14px;padding:12px;background:#faf7f2;border-radius:8px;user-select:all}}</style><main><div class="mark">COWCHAT</div><h1>Connect with {name}</h1><p>@{handle}</p><p>Use Cowchat to communicate with your existing agents. Open the app to preview this person and choose whether to send a friend request.</p><a class="button" href="{native}">Open Cowchat</a><p>New here? <a href="https://github.com/cowboyinc/cowchat/releases/latest">Download Cowchat for Mac</a>, install it, then return to this page and click Open Cowchat.</p><p class="link">{origin}/friends/{slug}</p><p>A friend request requires approval. This link does not grant access to rooms or agents.</p></main></html>"#
    );
    let mut response = axum::response::Html(html).into_response();
    let headers = response.headers_mut();
    headers.insert("content-security-policy", "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'".parse().unwrap());
    headers.insert("referrer-policy", "no-referrer".parse().unwrap());
    headers.insert("cache-control", "no-store".parse().unwrap());
    response
}
async fn action(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
    method: Method,
    body: Option<Json<Value>>,
) -> Response {
    let Some(config) = configuration(&state) else {
        return disabled();
    };
    let Some(key) = crate::web::authenticated_key(&state, &headers) else {
        return fail(anyhow::anyhow!("unauthorized"));
    };
    let parts = path.trim_matches('/').split('/').collect::<Vec<_>>();
    let data = body.map(|b| b.0).unwrap_or(json!({}));
    let revoking = matches!(
        (method.as_str(), parts.as_slice()),
        ("DELETE", ["credentials"] | ["agents", _])
    );
    let before_revoke = if revoking {
        state
            .store
            .social_principal(&key)
            .and_then(|principal| {
                state
                    .store
                    .social_live_session_states(&principal.account_id)
                    .ok()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let result = match (method.as_str(), parts.as_slice()) {
        ("GET", ["team"]) => state
            .store
            .social_team(&key, query.get("team_id").map(String::as_str)),
        ("POST", ["sessions"]) => {
            let field = |name: &str| {
                data.get(name)
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("invalid {name}"))
            };
            match (field("id"), field("name"), field("harness"), field("folder"), field("branch")) {
                (Ok(id), Ok(name), Ok(harness), Ok(folder), Ok(branch)) => state.store
                    .social_start_session(&key, id, name, harness, folder, branch)
                    .map(|room| {
                        if let Ok(event) = state.store.social_session_state_by_id(id) {
                            broadcast_session_state(&state.broker, &state.store, &event);
                        }
                        json!({"session_id":id,"room_id":room.room_id,"room_name":room.room_name,"session_token":room.session_token})
                    }),
                _ => Err(anyhow::anyhow!("invalid session fields")),
            }
        }
        ("POST", ["sessions", id, "heartbeat"]) => state
            .store
            .social_touch_session(&key, id)
            .map(|()| json!({"ok":true})),
        ("GET", ["sessions", id, "rooms"]) => state.store.social_session_rooms(&key, id),
        ("POST", ["sessions", id, "messages"]) => {
            let room = data["room_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("invalid room_id"));
            match (
                state.store.social_session_name(&key, id),
                state.store.social_display_name(&key, id),
                room,
            ) {
                (Ok(_), Some(name), Ok(room))
                    if state.store.social_room_access(room, &key) == Some(true) =>
                {
                    let frame = crate::handler::handle_send_message(
                        None,
                        data.clone(),
                        id,
                        &name,
                        &state.broker,
                        &state.store,
                        &key,
                        &state.rate_limiter,
                        false,
                        &state.webhook_mgr,
                        false,
                    )
                    .await;
                    if frame.frame_type == FrameType::Ok {
                        Ok(frame.payload)
                    } else {
                        Err(anyhow::anyhow!("message rejected: {}", frame.payload))
                    }
                }
                _ => Err(anyhow::anyhow!("not found")),
            }
        }
        ("POST", ["sessions", id, "presence"]) => match (
            state.store.social_session_name(&key, id),
            state.store.social_display_name(&key, id),
        ) {
            (Ok(_), Some(name)) => {
                let frame = crate::handler::handle_set_presence(
                    None,
                    data.clone(),
                    id,
                    &name,
                    &state.broker,
                    &state.store,
                    true,
                )
                .await;
                if frame.frame_type == FrameType::Ok {
                    let status = data["status"].as_str().unwrap_or("");
                    let session_state = match status {
                        "working" | "thinking" => Some("working"),
                        "prompt" => Some("prompt"),
                        "idle" => Some("idle"),
                        _ => None,
                    };
                    if let Some(status) = session_state {
                        if let Ok(event) = state.store.social_set_session_state(&key, status) {
                            broadcast_session_state(&state.broker, &state.store, &event);
                        }
                    }
                    Ok(frame.payload)
                } else {
                    Err(anyhow::anyhow!("presence rejected: {}", frame.payload))
                }
            }
            _ => Err(anyhow::anyhow!("not found")),
        },
        ("POST", ["sessions", id, "end"]) => {
            state.store.social_end_session(&key, id).map(|room_id| {
                if let Ok(event) = state.store.social_session_state_by_id(id) {
                    broadcast_session_state(&state.broker, &state.store, &event);
                }
                json!({"ok":true,"room_id":room_id})
            })
        }
        _ => state
            .store
            .social_action(&key, method.as_str(), &path, &data),
    };
    match result {
        Ok(mut value) => {
            for session in before_revoke {
                if state
                    .store
                    .social_session_state_by_id(&session.agent_id)
                    .is_ok_and(|current| current.state == "ended")
                {
                    let event = crate::store::social::SessionState {
                        state: "ended".into(),
                        ..session
                    };
                    broadcast_session_state(&state.broker, &state.store, &event);
                }
            }
            if path == "me/friend-link" {
                if let Some(slug) = value["slug"].as_str() {
                    value["url"] = json!(format!("{}/friends/{}", config.origin, slug));
                }
            }
            let approved_room = match (method.as_str(), parts.as_slice()) {
                ("POST", ["rooms", room, "agents"]) => Some(*room),
                ("POST", ["room-invitations", _, "accept"]) => value["room_id"].as_str(),
                _ => None,
            };
            if let Some(room) = approved_room {
                if let Ok(sessions) = state.store.social_live_sessions_in_room(room) {
                    for session in sessions {
                        state.broker.send_to_agent(
                            &session,
                            Frame::event(
                                FrameType::RoomApproved,
                                json!({"room_id":room,"agent_id":session}),
                            ),
                        );
                    }
                }
            }
            if method != Method::GET
                && !matches!(
                    parts.as_slice(),
                    ["sessions", _, "heartbeat" | "messages" | "presence"]
                )
            {
                for entry in state.broker.agents.iter() {
                    if entry.api_key.starts_with(crate::store::social::PREFIX) {
                        let invalid = state.store.social_principal(&entry.api_key).is_none()
                            || state.broker.room_members.iter().any(|members| {
                                members.contains(entry.key())
                                    && state
                                        .store
                                        .social_room_access(members.key(), &entry.api_key)
                                        != Some(true)
                            });
                        if invalid {
                            entry.send_task.abort();
                            entry.disconnect.notify_one();
                        }
                    }
                }
            }
            Json(value).into_response()
        }
        Err(e) => fail(e),
    }
}
#[derive(Deserialize)]
struct Start {
    provider: String,
    code_challenge: String,
    callback_port: u16,
}
async fn start(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Start>,
) -> Response {
    let Some(config) = configuration(&state) else {
        return disabled();
    };
    let Some(provider) = config.providers.get(&body.provider) else {
        return fail(anyhow::anyhow!("invalid provider"));
    };
    let source = crate::web::signup_bucket(peer, &headers, &state.trusted_proxy_ips);
    if !state.rate_limiter.try_register_session(&source) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":"Too many sign-in attempts"})),
        )
            .into_response();
    }
    let key = headers.get("x-cowchat-key").and_then(|h| h.to_str().ok());
    let (request_id, oauth_state) = match state.store.social_oauth_start(
        &body.provider,
        &body.code_challenge,
        key,
        body.callback_port,
        &source,
    ) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let mut url = url::Url::parse(&provider.authorize).unwrap();
    url.query_pairs_mut()
        .append_pair("client_id", &provider.id)
        .append_pair(
            "redirect_uri",
            &format!("{}/auth/social/{}/callback", config.origin, body.provider),
        )
        .append_pair("response_type", "code")
        .append_pair("state", &oauth_state)
        .append_pair(
            "scope",
            if body.provider == "google" {
                "openid profile"
            } else {
                "read:user"
            },
        );
    Json(json!({"request_id":request_id,"authorization_url":url.to_string(),"expires_in":600}))
        .into_response()
}
#[derive(Deserialize)]
struct Exchange {
    request_id: String,
    code_verifier: String,
    completion_code: String,
}
async fn exchange(State(state): State<AppState>, Json(body): Json<Exchange>) -> Response {
    if configuration(&state).is_none() {
        return disabled();
    }
    match state.store.social_oauth_exchange(
        &body.request_id,
        &body.code_verifier,
        &body.completion_code,
    ) {
        Ok(v) => Json(v).into_response(),
        Err(e) => fail(e),
    }
}
#[derive(Deserialize)]
struct Callback {
    state: String,
    code: Option<String>,
}
async fn callback(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<Callback>,
) -> Response {
    let Some(config) = configuration(&state) else {
        return disabled();
    };
    let Some(provider) = config.providers.get(&name) else {
        return fail(anyhow::anyhow!("invalid provider"));
    };
    let outcome = complete(&state, &config, &name, provider, &query).await;
    match outcome {
        Ok((port, code)) => {
            let mut response = axum::response::Redirect::to(&format!(
                "http://127.0.0.1:{port}/cowchat/oauth?request_id={}&completion_code={code}",
                query
                    .state
                    .split_once('.')
                    .expect("validated OAuth state")
                    .0
            ))
            .into_response();
            response.headers_mut().insert(
                "cache-control",
                axum::http::HeaderValue::from_static("no-store"),
            );
            response.headers_mut().insert(
                "referrer-policy",
                axum::http::HeaderValue::from_static("no-referrer"),
            );
            response
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            "Cowchat sign-in failed or expired. Start again in the app.",
        )
            .into_response(),
    }
}
async fn complete(
    state: &AppState,
    config: &Configuration,
    name: &str,
    p: &Provider,
    query: &Callback,
) -> anyhow::Result<(u16, String)> {
    let (request_id, secret) = query
        .state
        .split_once('.')
        .ok_or_else(|| anyhow::anyhow!("invalid state"))?;
    uuid::Uuid::parse_str(request_id)?;
    anyhow::ensure!(
        secret.len() == 64 && secret.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid state"
    );
    state.store.social_oauth_claim(name, &query.state)?;
    let code = query
        .code
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("provider rejected"))?;
    anyhow::ensure!(code.len() <= 4096, "invalid code");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("Cowchat")
        .build()?;
    let token: Value = client
        .post(&p.token)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", p.id.as_str()),
            ("client_secret", p.secret.as_str()),
            ("code", code),
            ("grant_type", "authorization_code"),
            (
                "redirect_uri",
                &format!("{}/auth/social/{name}/callback", config.origin),
            ),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let access = token["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("no token"))?;
    if let Some(tokeninfo) = &p.tokeninfo {
        let info: Value = client
            .get(tokeninfo)
            .query(&[("access_token", access)])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        anyhow::ensure!(info["aud"].as_str() == Some(&p.id), "wrong audience");
        let remaining = info["expires_in"]
            .as_i64()
            .or_else(|| info["expires_in"].as_str().and_then(|v| v.parse().ok()))
            .unwrap_or(0);
        anyhow::ensure!(remaining > 0, "expired token");
    }
    let user: Value = client
        .get(&p.user)
        .bearer_auth(access)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let subject = if name == "github" {
        user["id"].as_u64().map(|v| v.to_string())
    } else {
        user["sub"].as_str().map(str::to_owned)
    }
    .ok_or_else(|| anyhow::anyhow!("no subject"))?;
    let label = user["name"]
        .as_str()
        .or_else(|| user["login"].as_str())
        .unwrap_or("Cowchat user");
    state
        .store
        .social_oauth_complete(name, &query.state, &subject, label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        broker::Broker, rate_limit::RateLimiter, reconnect::ReconnectManager, store::Store,
        tasks::TaskManager, voting::VoteManager,
    };
    use base64::Engine;
    use cowchat_core::{Frame, FrameType};
    use dashmap::DashMap;
    use futures::{SinkExt, StreamExt};
    use sha2::{Digest, Sha256};
    use std::{collections::HashMap, sync::Arc};

    async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(
                socket,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });
        (format!("http://{addr}"), task)
    }
    fn state() -> AppState {
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
            session_auth: None,
        }
    }
    async fn fixture() -> (AppState, String, Vec<tokio::task::JoinHandle<()>>) {
        let mock=Router::new().route("/token",post(|axum::extract::Form(form):axum::extract::Form<HashMap<String,String>>|async move {Json(json!({"access_token":form["code"]}))})).route("/user",get(|headers:HeaderMap|async move {
            let token=headers["authorization"].to_str().unwrap().strip_prefix("Bearer ").unwrap();
            Json(json!({"id":token.parse::<u64>().unwrap(),"login":format!("user-{token}"),"sub":token}))
        })).route("/tokeninfo",get(|Query(q):Query<HashMap<String,String>>|async move {Json(json!({"aud":if q["access_token"]=="999"{"attacker-client"}else{"test-client"},"expires_in":3600}))}));
        let (mock_url, mock_task) = serve(mock).await;
        let state = state();
        let mut config = Configuration {
            origin: "https://chat.example".into(),
            providers: HashMap::new(),
        };
        for name in ["github", "google"] {
            config.providers.insert(
                name.into(),
                Provider {
                    id: "test-client".into(),
                    secret: "test-secret".into(),
                    authorize: format!("{mock_url}/authorize"),
                    token: format!("{mock_url}/token"),
                    user: format!("{mock_url}/user"),
                    tokeninfo: (name == "google").then(|| format!("{mock_url}/tokeninfo")),
                },
            );
        }
        state
            .store
            .social_configuration
            .set(Some(config))
            .ok()
            .unwrap();
        let (base, task) = serve(crate::web::router(state.clone())).await;
        (state, base, vec![task, mock_task])
    }

    #[tokio::test]
    async fn approved_session_receives_room_event_and_can_join() {
        let (_state, base, tasks) = fixture().await;
        let owner = login(&base, "github", "51", None).await;
        let owner_key = owner["device_key"].as_str().unwrap();
        let bot = call(&base, owner_key, "POST", "agents", json!({"name":"Worker"})).await;
        let bot_key = bot["api_key"].as_str().unwrap();
        let id = format!("session:{}", uuid::Uuid::new_v4());
        let session = call(
            &base,
            bot_key,
            "POST",
            "sessions",
            json!({"id":id,"name":"worker","harness":"codex","folder":"repo","branch":"main"}),
        )
        .await;
        let token = session["session_token"].as_str().unwrap();
        let ws_url = format!("{}/ws", base.replacen("http", "ws", 1));
        let client = cowchat_client::CowchatClient::connect_ws(
            &ws_url,
            token,
            "worker",
            Some(&id),
            vec![format!(
                "cowchat.session.v1:{}",
                json!({"harness":"codex","folder":"repo","branch":"main"})
            )],
        )
        .await
        .unwrap();
        let mut events = client.subscribe();
        let room = call(
            &base,
            owner_key,
            "POST",
            "rooms",
            json!({"name":"shared-work"}),
        )
        .await;
        let room_id = room["room_id"].as_str().unwrap();
        call(
            &base,
            owner_key,
            "POST",
            &format!("rooms/{room_id}/agents"),
            json!({"agent_ids":[id]}),
        )
        .await;
        let approved = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let event = events.recv().await.unwrap();
                if event.frame.frame_type == FrameType::RoomApproved {
                    break event.frame;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(approved.payload["room_id"], room_id);
        assert_eq!(approved.payload["agent_id"], id);
        client.join_room(room_id).await.unwrap();
        let listed = call(
            &base,
            token,
            "GET",
            &format!("sessions/{id}/rooms"),
            json!({}),
        )
        .await;
        assert!(listed["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|room| room["room_id"] == room_id));
        for task in tasks {
            task.abort();
        }
    }
    async fn call(base: &str, key: &str, method: &str, path: &str, body: Value) -> Value {
        let response = reqwest::Client::new()
            .request(method.parse().unwrap(), format!("{base}/social/v1/{path}"))
            .header("x-cowchat-key", key)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        let value: Value = serde_json::from_str(&body)
            .unwrap_or_else(|error| panic!("{path}: {status}: {error}: {body}"));
        assert_eq!(status, StatusCode::OK, "{path}: {value}");
        value
    }
    #[tokio::test]
    async fn team_rooms_require_explicit_kind_and_sharing_is_off_by_default() {
        let (_state, base, tasks) = fixture().await;
        let owner = login(&base, "github", "9301", None).await;
        let friend = login(&base, "github", "9302", None).await;
        let owner_key = owner["device_key"].as_str().unwrap();
        let friend_key = friend["device_key"].as_str().unwrap();
        let chat = call(
            &base,
            owner_key,
            "POST",
            "rooms",
            json!({"name":"three person chat"}),
        )
        .await;
        let team = call(
            &base,
            owner_key,
            "POST",
            "rooms",
            json!({"name":"Cowboy","kind":"team"}),
        )
        .await;
        let team_id = team["room_id"].as_str().unwrap();
        let listed = call(&base, owner_key, "GET", "teams", json!({})).await;
        assert_eq!(listed["teams"].as_array().unwrap().len(), 1);
        assert_eq!(listed["teams"][0]["id"], team_id);
        assert_ne!(listed["teams"][0]["id"], chat["room_id"]);
        let bot = call(
            &base,
            owner_key,
            "POST",
            "agents",
            json!({"name":"Worker","mint_only":true}),
        )
        .await;
        let id = format!("session:{}", uuid::Uuid::new_v4());
        let session = call(
            &base,
            bot["api_key"].as_str().unwrap(),
            "POST",
            "sessions",
            json!({"id":id,"name":"worker","harness":"claude","folder":"repo","branch":"main"}),
        )
        .await;
        call(&base, session["session_token"].as_str().unwrap(), "POST",
            &format!("sessions/{id}/messages"), json!({
                "room_id":session["room_id"],"content":"ready",
                "metadata":{"pull_request":{"repository":"cowboyinc/cowchat","number":91,
                    "title":"Finished work","address":"https://github.com/cowboyinc/cowchat/pull/91"}}
            })).await;
        let own = call(
            &base,
            owner_key,
            "GET",
            &format!("team?team_id={team_id}"),
            json!({}),
        )
        .await;
        assert_eq!(own["sharing"], false);
        assert!(own["people"].as_array().unwrap().is_empty());
        call(
            &base,
            owner_key,
            "PUT",
            "team/sharing",
            json!({"sharing":true}),
        )
        .await;
        let own = call(
            &base,
            owner_key,
            "GET",
            &format!("team?team_id={team_id}"),
            json!({}),
        )
        .await;
        assert_eq!(own["people"][0]["sessions"][0]["id"], id);
        assert!(own["people"][0]["sessions"][0]["room"].is_object());
        assert_eq!(
            own["people"][0]["sessions"][0]["pull_request"]["number"],
            91
        );
        let outsider = reqwest::Client::new()
            .get(format!("{base}/social/v1/team?team_id={team_id}"))
            .header("x-cowchat-key", friend_key)
            .send()
            .await
            .unwrap();
        assert_eq!(outsider.status(), StatusCode::NOT_FOUND);
        let request = call(
            &base,
            owner_key,
            "POST",
            "friend-requests",
            json!({"handle":friend["account"]["handle"]}),
        )
        .await;
        call(
            &base,
            friend_key,
            "POST",
            &format!(
                "friend-requests/{}/accept",
                request["request"]["id"].as_str().unwrap()
            ),
            json!({}),
        )
        .await;
        let invitation = call(
            &base,
            owner_key,
            "POST",
            &format!("rooms/{team_id}/invitations"),
            json!({"friend_id":friend["account"]["id"],"proposed_agent_ids":[]}),
        )
        .await;
        call(
            &base,
            friend_key,
            "POST",
            &format!(
                "room-invitations/{}/accept",
                invitation["id"].as_str().unwrap()
            ),
            json!({"agent_ids":[]}),
        )
        .await;
        let friend_team = call(
            &base,
            friend_key,
            "GET",
            &format!("team?team_id={team_id}"),
            json!({}),
        )
        .await;
        assert_eq!(friend_team["sharing"], false);
        assert_eq!(friend_team["people"][0]["sessions"][0]["id"], id);
        assert!(friend_team["people"][0]["sessions"][0]["room"].is_null());
        assert!(friend_team["people"][0]["sessions"][0]["pull_request"].is_null());
        for task in tasks {
            task.abort();
        }
    }

    #[tokio::test]
    async fn owner_receives_session_lifecycle_without_joining_its_room() {
        let (state, base, tasks) = fixture().await;
        let owner = login(&base, "github", "9201", None).await;
        let owner_key = owner["device_key"].as_str().unwrap();
        let ws_url = format!("{}/ws", base.replacen("http", "ws", 1));
        let owner_client =
            cowchat_client::CowchatClient::connect_ws(&ws_url, owner_key, "Owner", None, vec![])
                .await
                .unwrap();
        let mut events = owner_client.subscribe();
        let bot = call(
            &base,
            owner_key,
            "POST",
            "agents",
            json!({"name":"Worker","mint_only":true}),
        )
        .await;
        let id = format!("session:{}", uuid::Uuid::new_v4());
        let session = call(
            &base,
            bot["api_key"].as_str().unwrap(),
            "POST",
            "sessions",
            json!({"id":id,"name":"worker","harness":"claude","folder":"repo","branch":"main"}),
        )
        .await;
        let token = session["session_token"].as_str().unwrap();
        let room_id = session["room_id"].as_str().unwrap();
        async fn next_state(
            events: &mut tokio::sync::broadcast::Receiver<cowchat_client::Event>,
        ) -> Value {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let frame = events.recv().await.unwrap().frame;
                    if frame.frame_type == FrameType::SessionState {
                        break frame.payload;
                    }
                }
            })
            .await
            .unwrap()
        }
        let created = next_state(&mut events).await;
        assert_eq!(
            created,
            json!({"agent_id":id,"room_id":room_id,"state":"idle"})
        );
        call(
            &base,
            token,
            "POST",
            &format!("sessions/{id}/presence"),
            json!({"status":"prompt","status_detail":"permission"}),
        )
        .await;
        let prompt = next_state(&mut events).await;
        assert_eq!(prompt["state"], "prompt");
        let session_client = cowchat_client::CowchatClient::connect_ws(
            &ws_url,
            token,
            "worker",
            Some(&id),
            vec![format!(
                "cowchat.session.v1:{}",
                json!({"harness":"claude","folder":"repo","branch":"main"})
            )],
        )
        .await
        .unwrap();
        assert_eq!(next_state(&mut events).await["state"], "prompt");
        session_client
            .set_presence("waiting", None, None)
            .await
            .unwrap();
        assert_eq!(
            state.store.social_session_state_by_id(&id).unwrap().state,
            "prompt"
        );
        call(
            &base,
            token,
            "POST",
            &format!("sessions/{id}/end"),
            json!({}),
        )
        .await;
        let ended = next_state(&mut events).await;
        assert_eq!(ended["state"], "ended");
        let second_id = format!("session:{}", uuid::Uuid::new_v4());
        call(&base, bot["api_key"].as_str().unwrap(), "POST", "sessions",
            json!({"id":second_id,"name":"worker","harness":"claude","folder":"repo","branch":"main"})).await;
        assert_eq!(next_state(&mut events).await["state"], "idle");
        call(
            &base,
            owner_key,
            "DELETE",
            "credentials",
            json!({"api_key":bot["api_key"]}),
        )
        .await;
        let credential_ended = next_state(&mut events).await;
        assert_eq!(credential_ended["agent_id"], second_id);
        assert_eq!(credential_ended["state"], "ended");
        let next_bot = call(
            &base,
            owner_key,
            "POST",
            "agents",
            json!({"name":"Another Worker","mint_only":true}),
        )
        .await;
        let third_id = format!("session:{}", uuid::Uuid::new_v4());
        call(&base, next_bot["api_key"].as_str().unwrap(), "POST", "sessions",
            json!({"id":third_id,"name":"worker","harness":"claude","folder":"repo","branch":"main"})).await;
        assert_eq!(next_state(&mut events).await["state"], "idle");
        call(
            &base,
            owner_key,
            "DELETE",
            &format!("agents/{}", next_bot["agent"]["id"].as_str().unwrap()),
            json!({}),
        )
        .await;
        let bot_ended = next_state(&mut events).await;
        assert_eq!(bot_ended["agent_id"], third_id);
        assert_eq!(bot_ended["state"], "ended");
        for task in tasks {
            task.abort();
        }
    }

    #[tokio::test]
    async fn mint_only_credential_creates_session_over_http_but_cannot_operate_it() {
        let (state, base, tasks) = fixture().await;
        let owner = login(&base, "github", "9101", None).await;
        let owner_key = owner["device_key"].as_str().unwrap();
        let bot = call(
            &base,
            owner_key,
            "POST",
            "agents",
            json!({"name":"Cowbot","mint_only":true}),
        )
        .await;
        let mint_key = bot["api_key"].as_str().unwrap();
        let id = format!("session:{}", uuid::Uuid::new_v4());
        let session = call(
            &base,
            mint_key,
            "POST",
            "sessions",
            json!({"id":id,"name":"worker","harness":"codex","folder":"repo","branch":"main"}),
        )
        .await;
        let token = session["session_token"].as_str().unwrap();
        assert_eq!(session["session_id"], id);
        assert_eq!(
            state
                .store
                .social_room_access(session["room_id"].as_str().unwrap(), mint_key),
            Some(false)
        );
        let heartbeat = format!("sessions/{id}/heartbeat");
        let denied = reqwest::Client::new()
            .post(format!("{base}/social/v1/{heartbeat}"))
            .header("x-cowchat-key", mint_key)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::BAD_REQUEST);
        call(&base, token, "POST", &heartbeat, json!({})).await;
        let ws_url = format!("{}/ws", base.replacen("http", "ws", 1));
        let client = cowchat_client::CowchatClient::connect_ws(
            &ws_url,
            token,
            "worker",
            Some(&id),
            vec![format!(
                "cowchat.session.v1:{}",
                json!({"harness":"codex","folder":"repo","branch":"main"})
            )],
        )
        .await
        .unwrap();
        let room_id = session["room_id"].as_str().unwrap();
        client
            .send_message(room_id, "hello from session", None, vec![id.clone()])
            .await
            .unwrap();
        assert!(client
            .send_message(
                room_id,
                "invalid mention",
                None,
                vec!["session:outsider".into()]
            )
            .await
            .is_err());
        let history = client.get_history(room_id, 10, None).await.unwrap();
        assert_eq!(history.last().unwrap().content, "hello from session");
        assert_eq!(history.last().unwrap().metadata["mentions"], json!([id]));
        assert_eq!(history.last().unwrap().metadata["display_external"], false);
        for task in tasks {
            task.abort();
        }
    }
    async fn login(base: &str, provider: &str, code: &str, link: Option<&str>) -> Value {
        let verifier = "v".repeat(64);
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let mut request = client
            .post(format!("{base}/auth/social/start"))
            .json(&json!({"provider":provider,"code_challenge":challenge,"callback_port":49233}));
        if let Some(link) = link {
            request = request.header("x-cowchat-key", link);
        }
        let start: Value = request.send().await.unwrap().json().await.unwrap();
        let auth = url::Url::parse(start["authorization_url"].as_str().unwrap()).unwrap();
        let oauth_state = auth
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .into_owned();
        let callback = client
            .get(format!("{base}/auth/social/{provider}/callback"))
            .query(&[("state", oauth_state.as_str()), ("code", code)])
            .send()
            .await
            .unwrap();
        assert_eq!(callback.status(), StatusCode::SEE_OTHER);
        let location = url::Url::parse(callback.headers()["location"].to_str().unwrap()).unwrap();
        if let Ok(path) = std::env::var("COWCHAT_SOCIAL_CALLBACK_FIXTURE") {
            use std::io::Write;
            match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(mut file)=>file.write_all(&serde_json::to_vec(&json!({"request_id":start["request_id"],"callback_url":location.to_string(),"callback_port":49233})).unwrap()).unwrap(),
                Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>{},
                Err(error)=>panic!("callback fixture could not be written: {error}"),
            }
        }
        assert_eq!(
            location
                .query_pairs()
                .find(|(k, _)| k == "request_id")
                .unwrap()
                .1,
            start["request_id"].as_str().unwrap()
        );
        assert_eq!(location.host_str(), Some("127.0.0.1"));
        assert_eq!(location.port(), Some(49233));
        let completion = location
            .query_pairs()
            .find(|(k, _)| k == "completion_code")
            .unwrap()
            .1
            .into_owned();
        let wrong=client.post(format!("{base}/auth/social/exchange")).json(&json!({"request_id":start["request_id"],"code_verifier":"wrong".repeat(16),"completion_code":completion})).send().await.unwrap();
        assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
        let body = json!({"request_id":start["request_id"],"code_verifier":verifier,"completion_code":completion});
        let response = client
            .post(format!("{base}/auth/social/exchange"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result: Value = response.json().await.unwrap();
        let replay = client
            .post(format!("{base}/auth/social/exchange"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
        result
    }
    type Socket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;
    async fn frame(socket: &mut Socket, kind: FrameType, payload: Value) -> Frame {
        let id = uuid::Uuid::new_v4().to_string();
        let request = Frame {
            id: Some(id.clone()),
            reply_to: None,
            frame_type: kind,
            payload,
        };
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                request.to_line().unwrap().into(),
            ))
            .await
            .unwrap();
        loop {
            let value = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let tokio_tungstenite::tungstenite::Message::Text(text) = value {
                let response = Frame::from_line(&text).unwrap();
                if response.reply_to.as_deref() == Some(&id) {
                    return response;
                }
            }
        }
    }
    async fn assert_no_marker(socket: &mut Socket, marker: &str) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(200);
        loop {
            match tokio::time::timeout_at(deadline, socket.next()).await {
                Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))) => {
                    assert!(!text.contains(marker), "Unauthorized delivery: {text}")
                }
                Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))))
                | Ok(None)
                | Ok(Some(Err(_)))
                | Err(_) => break,
                _ => {}
            }
        }
    }
    async fn connect(base: &str, key: &str, id: &str) -> (Socket, Frame) {
        let (mut socket, _) =
            tokio_tungstenite::connect_async(format!("{}/ws", base.replacen("http", "ws", 1)))
                .await
                .unwrap();
        let response=frame(&mut socket,FrameType::Register,json!({"name":"test-agent","agent_id":id,"key":key,"capabilities":[],"reconnect":true,"protocol_version":cowchat_core::PROTOCOL_VERSION})).await;
        (socket, response)
    }
    #[tokio::test]
    async fn invitation_landing_and_handle_endpoints_are_safe() {
        let (_state, base, tasks) = fixture().await;
        let alice = login(&base, "github", "801", None).await;
        let bob = login(&base, "github", "802", None).await;
        let ak = alice["device_key"].as_str().unwrap();
        let bk = bob["device_key"].as_str().unwrap();
        let link = call(&base, ak, "POST", "me/friend-link", json!({})).await;
        assert_eq!(
            link["url"],
            format!(
                "https://chat.example/friends/{}",
                link["slug"].as_str().unwrap()
            )
        );
        call(
            &base,
            ak,
            "PATCH",
            "me",
            json!({"handle":"safe-handle","display_name":"<script>alert(1)</script> & \"cow\""}),
        )
        .await;
        let client = reqwest::Client::new();
        let landing = client
            .get(format!("{base}/friends/{}", link["slug"].as_str().unwrap()))
            .header("host", "evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(landing.status(), StatusCode::OK);
        assert_eq!(landing.headers()["referrer-policy"], "no-referrer");
        assert_eq!(landing.headers()["cache-control"], "no-store");
        assert!(landing.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("default-src 'none'"));
        let html = landing.text().await.unwrap();
        if let Ok(path) = std::env::var("COWCHAT_FRIEND_LINK_FIXTURE") {
            std::fs::write(path, &html).unwrap();
        }
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("cowchat://friend/"));
        assert!(html.contains("server=wss%3A%2F%2Fchat.example%2Fws"));
        assert!(!html.contains("evil.example"));
        assert!(html.contains("https://github.com/cowboyinc/cowchat/releases/latest"));
        assert!(!html.contains(ak));
        assert!(
            call(&base, ak, "GET", "friend-requests", json!({})).await["requests"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            client
                .get(format!("{base}/friends/missing"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        let agent = call(
            &base,
            ak,
            "POST",
            "agents",
            json!({"name":"bot","discoverable":false}),
        )
        .await;
        let issued = call(
            &base,
            ak,
            "POST",
            &format!(
                "agents/{}/credentials",
                agent["agent"]["id"].as_str().unwrap()
            ),
            json!({}),
        )
        .await;
        let bot = issued["api_key"].as_str().unwrap();
        for key in ["", bot] {
            let status = client
                .post(format!("{base}/social/v1/me/friend-link"))
                .header("x-cowchat-key", key)
                .json(&json!({}))
                .send()
                .await
                .unwrap()
                .status();
            assert!(status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN);
        }
        let conflict = client
            .patch(format!("{base}/social/v1/me"))
            .header("x-cowchat-key", bk)
            .json(&json!({"handle":"SAFE-HANDLE","display_name":"Bob"}))
            .send()
            .await
            .unwrap();
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        assert_eq!(
            conflict.json::<Value>().await.unwrap(),
            json!({"error":"handle_unavailable"})
        );
        let caps = client
            .get(format!("{base}/social/v1/capabilities"))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(caps["setup_version"], 1);
        assert_eq!(caps["friend_links"], true);
        assert_eq!(caps["handle_availability"], true);
        assert_eq!(caps["setup_handle"], true);
        for task in tasks {
            task.abort();
        }
    }
    #[tokio::test]
    async fn starter_setup_and_credentials_are_private_over_http_and_websocket() {
        let (state, base, tasks) = fixture().await;
        let alice = login(&base, "github", "1", None).await;
        let bob = login(&base, "google", "2", None).await;
        let ak = alice["device_key"].as_str().unwrap();
        let bk = bob["device_key"].as_str().unwrap();
        let client = reqwest::Client::new();
        let capabilities: Value = client
            .get(format!("{base}/social/v1/capabilities"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(capabilities["setup_version"], 1);
        assert_eq!(
            call(&base, ak, "GET", "setup", json!({})).await,
            json!({"setup":null})
        );
        let initial = call(
            &base,
            ak,
            "POST",
            "setup",
            json!({"display_name":"Chad DePue"}),
        )
        .await;
        assert!(!initial.to_string().contains("api_key"));
        assert_eq!(call(&base, ak, "POST", "setup", json!({})).await, initial);
        let room = initial["setup"]["room_id"].as_str().unwrap();
        let agent = initial["setup"]["agent_id"].as_str().unwrap();
        let endpoint = format!("agents/{agent}/credentials");
        let issued = call(&base, ak, "POST", &endpoint, json!({})).await;
        let bot_key = issued["api_key"].as_str().unwrap();
        for (key, path, method) in [
            (bk, endpoint.as_str(), "POST"),
            (bot_key, endpoint.as_str(), "POST"),
            (bot_key, "setup", "GET"),
            (bot_key, "setup", "POST"),
        ] {
            let denied = client
                .request(method.parse().unwrap(), format!("{base}/social/v1/{path}"))
                .header("x-cowchat-key", key)
                .json(&json!({}))
                .send()
                .await
                .unwrap();
            assert!(!denied.status().is_success());
        }
        let (mut bot, ok) = connect(&base, bot_key, agent).await;
        assert_eq!(ok.frame_type, FrameType::Ok);
        assert_eq!(
            frame(&mut bot, FrameType::JoinRoom, json!({"room_id":room}))
                .await
                .frame_type,
            FrameType::Ok
        );
        let (mut stranger, ok) = connect(&base, bk, "stranger-device").await;
        assert_eq!(ok.frame_type, FrameType::Ok);
        assert_eq!(
            frame(&mut stranger, FrameType::JoinRoom, json!({"room_id":room}))
                .await
                .frame_type,
            FrameType::Error
        );
        for operation in [FrameType::RoomInfo, FrameType::GetHistory] {
            assert_eq!(
                frame(&mut stranger, operation, json!({"room_id":room}))
                    .await
                    .frame_type,
                FrameType::Error
            );
        }
        let visible = frame(&mut stranger, FrameType::ListRooms, json!({})).await;
        assert!(!visible.payload.to_string().contains("Chad_DePue_Home"));
        let unrelated = call(&base, bk, "POST", "agents", json!({"name":"unrelated"})).await;
        let (mut stranger_bot, ok) = connect(
            &base,
            unrelated["api_key"].as_str().unwrap(),
            unrelated["agent"]["id"].as_str().unwrap(),
        )
        .await;
        assert_eq!(ok.frame_type, FrameType::Ok);
        assert_eq!(
            frame(
                &mut stranger_bot,
                FrameType::JoinRoom,
                json!({"room_id":room})
            )
            .await
            .frame_type,
            FrameType::Error
        );
        assert_eq!(
            frame(
                &mut stranger_bot,
                FrameType::RoomInfo,
                json!({"room_id":room})
            )
            .await
            .frame_type,
            FrameType::Error
        );
        for path in [
            format!("/api/rooms/{room}/history"),
            format!("/api/rooms/{room}/artifact"),
            format!("/social/v1/rooms/{room}/participants"),
        ] {
            let denied = client
                .get(format!("{base}{path}"))
                .header("x-cowchat-key", bk)
                .send()
                .await
                .unwrap();
            assert_eq!(denied.status(), StatusCode::NOT_FOUND);
        }
        let listed: Value = client
            .get(format!("{base}/api/rooms"))
            .header("x-cowchat-key", bk)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(!listed.to_string().contains("Chad_DePue_Home"));
        assert_eq!(
            call(&base, bk, "GET", "rooms", json!({})).await["rooms"],
            json!([])
        );
        state.store.social_assert_hash_only();
        for task in tasks {
            task.abort();
        }
    }
    #[tokio::test]
    async fn oauth_friends_room_consent_and_revocation_over_http_and_websocket() {
        let (state, base, tasks) = fixture().await;
        let alice = login(&base, "github", "1", None).await;
        let bob = login(&base, "google", "2", None).await;
        let ak = alice["device_key"].as_str().unwrap();
        let bk = bob["device_key"].as_str().unwrap();
        let linked = login(&base, "google", "3", Some(ak)).await;
        assert_eq!(linked["account"]["id"], alice["account"]["id"]);
        let aa = call(
            &base,
            ak,
            "POST",
            "agents",
            json!({"name":"Alice coordinator","discoverable":true}),
        )
        .await;
        let ab = call(
            &base,
            ak,
            "POST",
            "agents",
            json!({"name":"Alice sibling","discoverable":true}),
        )
        .await;
        let ba = call(
            &base,
            bk,
            "POST",
            "agents",
            json!({"name":"Bob coordinator","discoverable":true}),
        )
        .await;
        let request = call(
            &base,
            ak,
            "POST",
            "friend-requests",
            json!({"handle":bob["account"]["handle"]}),
        )
        .await;
        let premature=state.store.social_action(ak,"POST","rooms",&json!({"name":"premature","friend_id":bob["account"]["id"],"agent_ids":[aa["agent"]["id"]]}));
        assert!(premature.is_err());
        let id = request["request"]["id"].as_str().unwrap();
        assert!(state
            .store
            .social_action(
                aa["api_key"].as_str().unwrap(),
                "POST",
                &format!("friend-requests/{id}/accept"),
                &json!({})
            )
            .is_err());
        call(
            &base,
            bk,
            "POST",
            &format!("friend-requests/{id}/accept"),
            json!({}),
        )
        .await;
        let room=call(&base,ak,"POST","rooms",json!({"name":"safe collaboration","friend_id":bob["account"]["id"],"agent_ids":[aa["agent"]["id"]],"proposed_agent_ids":[ba["agent"]["id"]]})).await;
        let rid = room["room_id"].as_str().unwrap();
        assert_eq!(state.store.social_room_access(rid, bk), Some(false));
        assert_eq!(state.store.social_room_access(rid, "master"), Some(false));
        assert_eq!(
            state
                .store
                .social_room_access(rid, ab["api_key"].as_str().unwrap()),
            Some(false)
        );
        let inv = room["invitation"]["id"].as_str().unwrap();
        assert!(state
            .store
            .social_action(
                bk,
                "POST",
                &format!("room-invitations/{inv}/accept"),
                &json!({"agent_ids":[aa["agent"]["id"]]})
            )
            .is_err());
        call(
            &base,
            bk,
            "POST",
            &format!("room-invitations/{inv}/accept"),
            json!({"agent_ids":[ba["agent"]["id"]]}),
        )
        .await;
        let (mut a, ok) = connect(
            &base,
            aa["api_key"].as_str().unwrap(),
            aa["agent"]["id"].as_str().unwrap(),
        )
        .await;
        assert_eq!(ok.frame_type, FrameType::Ok);
        let (mut b, ok) = connect(
            &base,
            ba["api_key"].as_str().unwrap(),
            ba["agent"]["id"].as_str().unwrap(),
        )
        .await;
        assert_eq!(ok.frame_type, FrameType::Ok);
        for socket in [&mut a, &mut b] {
            assert_eq!(
                frame(socket, FrameType::JoinRoom, json!({"room_id":rid}))
                    .await
                    .frame_type,
                FrameType::Ok
            );
        }
        assert_eq!(
            frame(
                &mut a,
                FrameType::SendMessage,
                json!({"room_id":rid,"content":"approved hello"})
            )
            .await
            .frame_type,
            FrameType::Ok
        );
        let (_, wrong) = connect(&base, ak, aa["agent"]["id"].as_str().unwrap()).await;
        assert_eq!(wrong.frame_type, FrameType::Error);
        let (_, wrong) = connect(&base, aa["api_key"].as_str().unwrap(), "wrong-id").await;
        assert_eq!(wrong.frame_type, FrameType::Error);
        let (mut sibling, ok) = connect(
            &base,
            ab["api_key"].as_str().unwrap(),
            ab["agent"]["id"].as_str().unwrap(),
        )
        .await;
        assert_eq!(ok.frame_type, FrameType::Ok);
        assert_eq!(
            frame(&mut sibling, FrameType::JoinRoom, json!({"room_id":rid}))
                .await
                .frame_type,
            FrameType::Error
        );
        assert_eq!(
            frame(&mut a, FrameType::SubscribeActor, json!({"room_id":rid}))
                .await
                .frame_type,
            FrameType::Error
        );
        assert_eq!(
            frame(
                &mut a,
                FrameType::CreateRoom,
                json!({"name":"bypass","public":true})
            )
            .await
            .frame_type,
            FrameType::Error
        );
        loop {
            let message = tokio::time::timeout(std::time::Duration::from_secs(3), b.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
                if text.contains("approved hello") {
                    break;
                }
            }
        }
        let expected_name = format!(
            "Alice coordinator · @{}",
            alice["account"]["handle"].as_str().unwrap()
        );
        assert_eq!(
            state
                .broker
                .agents
                .get(aa["agent"]["id"].as_str().unwrap())
                .unwrap()
                .info
                .name,
            expected_name
        );
        assert_eq!(
            state.store.get_history(rid, 100, None).unwrap()[0].agent_name,
            expected_name
        );
        let client = reqwest::Client::new();
        let artifact = json!({"title":"Private board","view":"root = Stack([])","data":{},"request_id":uuid::Uuid::new_v4()});
        assert_eq!(
            client
                .put(format!("{base}/api/rooms/{rid}/artifact"))
                .header("x-cowchat-key", ak)
                .json(&artifact)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        let blob: Value = client
            .post(format!("{base}/api/rooms/{rid}/blobs?name=private.txt"))
            .header("x-cowchat-key", ak)
            .body("private attachment")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let blob_id = blob["blob_id"].as_str().unwrap();
        let other = call(
            &base,
            ak,
            "POST",
            "rooms",
            json!({"name":"different approval","friend_id":bob["account"]["id"],"agent_ids":[]}),
        )
        .await;
        let other_id = other["room_id"].as_str().unwrap();
        let decline_id = other["invitation"]["id"].as_str().unwrap();
        call(
            &base,
            bk,
            "POST",
            &format!("room-invitations/{decline_id}/decline"),
            json!({}),
        )
        .await;

        state
            .task_mgr
            .create_task(
                "private-task".into(),
                other_id.into(),
                "private".into(),
                None,
                None,
                "owner".into(),
            )
            .unwrap();
        state
            .store
            .create_vote(
                "private-vote",
                other_id,
                "private",
                None,
                &["yes".into(), "no".into()],
                "owner",
                None,
                &[],
            )
            .unwrap();
        assert_eq!(
            frame(
                &mut a,
                FrameType::UpdateTask,
                json!({"room_id":rid,"task_id":"private-task","status":"done"})
            )
            .await
            .frame_type,
            FrameType::Error
        );
        assert_eq!(
            frame(
                &mut a,
                FrameType::GetVoteStatus,
                json!({"room_id":rid,"vote_id":"private-vote"})
            )
            .await
            .frame_type,
            FrameType::Error
        );
        call(
            &base,
            ak,
            "DELETE",
            &format!("rooms/{rid}/agents/{}", ba["agent"]["id"].as_str().unwrap()),
            json!({}),
        )
        .await;
        assert_eq!(
            state
                .store
                .social_room_access(rid, ba["api_key"].as_str().unwrap()),
            Some(false)
        );
        for path in [
            format!("api/rooms/{rid}/artifact"),
            format!("api/blobs/{blob_id}"),
        ] {
            assert_eq!(
                client
                    .get(format!("{base}/{path}"))
                    .header("x-cowchat-key", ba["api_key"].as_str().unwrap())
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NOT_FOUND
            );
            assert_eq!(
                client
                    .get(format!("{base}/{path}"))
                    .header("x-cowchat-key", "master")
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NOT_FOUND
            );
        }
        assert_eq!(
            frame(
                &mut a,
                FrameType::SendMessage,
                json!({"room_id":rid,"content":"after removal"})
            )
            .await
            .frame_type,
            FrameType::Ok
        );
        assert_no_marker(&mut b, "after removal").await;
        let (mut reconnect, ok) = connect(
            &base,
            ba["api_key"].as_str().unwrap(),
            ba["agent"]["id"].as_str().unwrap(),
        )
        .await;
        assert_eq!(ok.frame_type, FrameType::Ok);
        assert_eq!(
            ok.payload
                .get("missed_messages")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            0
        );
        assert_eq!(
            frame(
                &mut reconnect,
                FrameType::GetHistory,
                json!({"room_id":rid})
            )
            .await
            .frame_type,
            FrameType::Error
        );
        call(
            &base,
            ak,
            "DELETE",
            &format!("agents/{}", aa["agent"]["id"].as_str().unwrap()),
            json!({}),
        )
        .await;
        let (_, denied) = connect(
            &base,
            aa["api_key"].as_str().unwrap(),
            aa["agent"]["id"].as_str().unwrap(),
        )
        .await;
        assert_eq!(denied.frame_type, FrameType::Error);
        call(
            &base,
            bk,
            "DELETE",
            &format!("rooms/{rid}/membership"),
            json!({}),
        )
        .await;
        let hidden = call(
            &base,
            ak,
            "POST",
            "agents",
            json!({"name":"private agent added later","discoverable":false}),
        )
        .await;
        call(
            &base,
            ak,
            "POST",
            &format!("rooms/{rid}/agents"),
            json!({"agent_ids":[hidden["agent"]["id"]]}),
        )
        .await;
        call(
            &base,
            ak,
            "POST",
            &format!("rooms/{other_id}/agents"),
            json!({"agent_ids":[hidden["agent"]["id"]]}),
        )
        .await;
        let invitations = call(&base, bk, "GET", "room-invitations", json!({})).await;
        assert!(invitations["invitations"].as_array().unwrap().is_empty());
        for room in [rid, other_id] {
            let denied = client
                .get(format!("{base}/social/v1/rooms/{room}/participants"))
                .header("x-cowchat-key", bk)
                .send()
                .await
                .unwrap();
            assert_eq!(denied.status(), StatusCode::NOT_FOUND);
        }
        state.store.social_assert_hash_only();
        for task in tasks {
            task.abort();
        }
    }
    #[tokio::test]
    async fn oauth_rejects_wrong_audience_state_replay_and_rate_exhaustion() {
        let (state, base, tasks) = fixture().await;
        let verifier = "q".repeat(64);
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let start: Value = client
            .post(format!("{base}/auth/social/start"))
            .json(&json!({"provider":"google","code_challenge":challenge,"callback_port":49233}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let url = url::Url::parse(start["authorization_url"].as_str().unwrap()).unwrap();
        let oauth_state = url
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .into_owned();
        let bad = client
            .get(format!("{base}/auth/social/google/callback"))
            .query(&[("state", "malformed"), ("code", "2")])
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        let wrong = client
            .get(format!("{base}/auth/social/google/callback"))
            .query(&[("state", oauth_state.as_str()), ("code", "999")])
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
        let replay = client
            .get(format!("{base}/auth/social/google/callback"))
            .query(&[("state", oauth_state.as_str()), ("code", "2")])
            .send()
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
        let stolen=client.post(format!("{base}/auth/social/exchange")).json(&json!({"request_id":start["request_id"],"code_verifier":verifier,"completion_code":"guessed"})).send().await.unwrap();
        assert_eq!(stolen.status(), StatusCode::BAD_REQUEST);
        let mut limited = false;
        for _ in 0..25 {
            let response = client
                .post(format!("{base}/auth/social/start"))
                .header("x-forwarded-for", uuid::Uuid::new_v4().to_string())
                .json(
                    &json!({"provider":"github","code_challenge":challenge,"callback_port":49233}),
                )
                .send()
                .await
                .unwrap();
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                limited = true;
                break;
            }
            assert_eq!(response.status(), StatusCode::OK);
        }
        assert!(
            limited,
            "one source must not fill all pending sign-in slots"
        );
        assert!(state
            .store
            .social_action("csoc_invalid", "GET", "me", &json!({}))
            .is_err());
        for task in tasks {
            task.abort();
        }
    }
    #[tokio::test]
    async fn no_auth_master_public_and_logout_never_override_social_authority() {
        let (state, base, tasks) = fixture().await;
        let alice = login(&base, "github", "11", None).await;
        let bob = login(&base, "github", "12", None).await;
        let ak = alice["device_key"].as_str().unwrap();
        let bk = bob["device_key"].as_str().unwrap();
        let agent = call(&base, ak, "POST", "agents", json!({"name":"reserved"})).await;
        let request = call(
            &base,
            ak,
            "POST",
            "friend-requests",
            json!({"handle":bob["account"]["handle"]}),
        )
        .await;
        let request_id = request["request"]["id"].as_str().unwrap();
        assert!(state
            .store
            .social_action(
                ak,
                "POST",
                &format!("friend-requests/{request_id}/accept"),
                &json!({})
            )
            .is_err());
        call(
            &base,
            bk,
            "POST",
            &format!("friend-requests/{request_id}/accept"),
            json!({}),
        )
        .await;
        let room=call(&base,ak,"POST","rooms",json!({"name":"private social","friend_id":bob["account"]["id"],"agent_ids":[agent["agent"]["id"]]})).await;
        let rid = room["room_id"].as_str().unwrap();
        let mut no_auth = state.clone();
        no_auth.no_auth = true;
        let (open, task) = serve(crate::web::router(no_auth)).await;
        let (_, invalid) = connect(&open, "csoc_invalid", "random").await;
        assert_eq!(invalid.frame_type, FrameType::Error);
        let (_, impersonation) =
            connect(&open, "anything", agent["agent"]["id"].as_str().unwrap()).await;
        assert_eq!(impersonation.frame_type, FrameType::Error);
        let (mut master, ok) = connect(&open, "master", "legacy").await;
        assert_eq!(ok.frame_type, FrameType::Ok);
        assert_eq!(
            frame(&mut master, FrameType::JoinRoom, json!({"room_id":rid}))
                .await
                .frame_type,
            FrameType::Error
        );
        let client = reqwest::Client::new();
        for key in ["master", "csoc_invalid", ""] {
            let response = client
                .get(format!("{open}/api/rooms/{rid}/history"))
                .header("x-cowchat-key", key)
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if key.starts_with("csoc_") {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::NOT_FOUND
                }
            );
        }
        let rooms = client
            .get(format!("{open}/api/rooms"))
            .header("x-cowchat-key", "csoc_invalid")
            .send()
            .await
            .unwrap();
        assert_eq!(rooms.status(), StatusCode::UNAUTHORIZED);
        state
            .store
            .create_room_with_visibility(
                "legacy-public",
                "Legacy public",
                None,
                None,
                Some("legacy"),
                "public",
                Some("master"),
                false,
            )
            .unwrap();
        assert_eq!(
            frame(
                &mut master,
                FrameType::JoinRoom,
                json!({"room_id":"legacy-public"})
            )
            .await
            .frame_type,
            FrameType::Ok
        );
        let (mut owner, ok) = connect(&base, ak, "human-device").await;
        assert_eq!(ok.frame_type, FrameType::Ok);
        assert_eq!(
            frame(&mut owner, FrameType::JoinRoom, json!({"room_id":rid}))
                .await
                .frame_type,
            FrameType::Ok
        );
        assert_eq!(
            frame(
                &mut owner,
                FrameType::RenameRoom,
                json!({"room_id":rid,"name":"Renamed social"})
            )
            .await
            .frame_type,
            FrameType::Ok
        );
        call(&base, ak, "POST", "logout", json!({})).await;
        let (_, revoked) = connect(&open, ak, "human-device").await;
        assert_eq!(revoked.frame_type, FrameType::Error);
        assert!(state.store.social_principal(ak).is_none());
        state.broker.broadcast_to_room_all(
            rid,
            &Frame::event(
                FrameType::MessageReceived,
                json!({"room_id":rid,"content":"after logout marker"}),
            ),
        );
        assert_no_marker(&mut owner, "after logout marker").await;
        task.abort();
        for task in tasks {
            task.abort();
        }
    }
    #[tokio::test]
    async fn managed_room_destroy_notifies_exact_previous_audience() {
        let (state, base, tasks) = fixture().await;
        let alice = login(&base, "github", "31", None).await;
        let bob = login(&base, "github", "32", None).await;
        let ak = alice["device_key"].as_str().unwrap();
        let bk = bob["device_key"].as_str().unwrap();
        let request = call(
            &base,
            ak,
            "POST",
            "friend-requests",
            json!({"handle":bob["account"]["handle"]}),
        )
        .await;
        call(
            &base,
            bk,
            "POST",
            &format!(
                "friend-requests/{}/accept",
                request["request"]["id"].as_str().unwrap()
            ),
            json!({}),
        )
        .await;
        let room = call(
            &base,
            ak,
            "POST",
            "rooms",
            json!({"name":"destroy test","friend_id":bob["account"]["id"],"agent_ids":[]}),
        )
        .await;
        let rid = room["room_id"].as_str().unwrap();
        call(
            &base,
            bk,
            "POST",
            &format!(
                "room-invitations/{}/accept",
                room["invitation"]["id"].as_str().unwrap()
            ),
            json!({"agent_ids":[]}),
        )
        .await;
        let (mut owner, ok) = connect(&base, ak, "owner-device").await;
        assert_eq!(ok.frame_type, FrameType::Ok);
        let (mut friend, ok) = connect(&base, bk, "friend-device").await;
        assert_eq!(ok.frame_type, FrameType::Ok);
        let (mut outsider, ok) = connect(&base, "master", "outsider").await;
        assert_eq!(ok.frame_type, FrameType::Ok);
        for socket in [&mut owner, &mut friend] {
            assert_eq!(
                frame(socket, FrameType::JoinRoom, json!({"room_id":rid}))
                    .await
                    .frame_type,
                FrameType::Ok
            );
        }
        let command = Frame {
            id: Some("destroy-command".into()),
            reply_to: None,
            frame_type: FrameType::DestroyRoom,
            payload: json!({"room_id":rid}),
        };
        owner
            .send(tokio_tungstenite::tungstenite::Message::Text(
                command.to_line().unwrap().into(),
            ))
            .await
            .unwrap();
        let (mut destroyed, mut acknowledged) = (false, false);
        while !destroyed || !acknowledged {
            let event = tokio::time::timeout(std::time::Duration::from_secs(3), owner.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let tokio_tungstenite::tungstenite::Message::Text(text) = event {
                let f = Frame::from_line(&text).unwrap();
                destroyed |= f.frame_type == FrameType::RoomDestroyed;
                acknowledged |= f.reply_to.as_deref() == Some("destroy-command")
                    && f.frame_type == FrameType::Ok;
            }
        }
        loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(3), friend.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let tokio_tungstenite::tungstenite::Message::Text(text) = event {
                let f = Frame::from_line(&text).unwrap();
                if f.frame_type == FrameType::RoomDestroyed {
                    assert_eq!(f.payload["room_id"], rid);
                    break;
                }
            }
        }
        assert_no_marker(&mut outsider, rid).await;
        assert!(call(&base, bk, "GET", "rooms", json!({})).await["rooms"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(state.store.get_room(rid).unwrap().is_none());
        for task in tasks {
            task.abort();
        }
    }

    #[tokio::test]
    async fn bot_sessions_register_into_separate_rooms() {
        let (state, base, tasks) = fixture().await;
        let owner = login(&base, "github", "9001", None).await;
        let owner_key = owner["device_key"].as_str().unwrap();
        let bot = call(&base, owner_key, "POST", "agents", json!({"name":"Cowbot"})).await;
        let bot_key = bot["api_key"].as_str().unwrap();
        let first_id = format!("session:{}", uuid::Uuid::new_v4());
        let second_id = format!("session:{}", uuid::Uuid::new_v4());

        let register = |id: &str, name: &str, branch: &str| {
            json!({
                "key": bot_key,
                "agent_id": id,
                "name": name,
                "reconnect": true,
                "protocol_version": cowchat_core::PROTOCOL_VERSION,
                "capabilities": [format!("cowchat.session.v1:{}", json!({
                    "harness": "claude",
                    "folder": "cowchat",
                    "branch": branch,
                }))]
            })
        };
        let (mut first, _) =
            tokio_tungstenite::connect_async(format!("{}/ws", base.replacen("http", "ws", 1)))
                .await
                .unwrap();
        let first_ok = frame(
            &mut first,
            FrameType::Register,
            register(&first_id, "claude-one", "main"),
        )
        .await;
        assert_eq!(first_ok.frame_type, FrameType::Ok, "{first_ok:?}");
        let first_room = first_ok.payload["session_room"]["room_id"]
            .as_str()
            .unwrap();
        assert_eq!(first_ok.payload["session_room"]["name"], "cowchat");
        let first_token = first_ok.payload["session_token"].as_str().unwrap();
        assert!(state.broker.is_agent_in_room(&first_id, first_room));

        let (mut impostor, _) =
            tokio_tungstenite::connect_async(format!("{}/ws", base.replacen("http", "ws", 1)))
                .await
                .unwrap();
        assert_eq!(
            frame(
                &mut impostor,
                FrameType::Register,
                register(&first_id, "impostor", "main")
            )
            .await
            .frame_type,
            FrameType::Error
        );
        assert!(state.broker.is_agent_in_room(&first_id, first_room));

        let (mut second, _) =
            tokio_tungstenite::connect_async(format!("{}/ws", base.replacen("http", "ws", 1)))
                .await
                .unwrap();
        let second_ok = frame(
            &mut second,
            FrameType::Register,
            register(&second_id, "codex-two", "feature"),
        )
        .await;
        assert_eq!(second_ok.frame_type, FrameType::Ok, "{second_ok:?}");
        let second_room = second_ok.payload["session_room"]["room_id"]
            .as_str()
            .unwrap();
        assert_ne!(first_room, second_room);
        assert_eq!(second_ok.payload["session_room"]["name"], "feature");
        assert_eq!(
            frame(
                &mut first,
                FrameType::SendMessage,
                json!({"room_id":first_room,"content":"private work"})
            )
            .await
            .frame_type,
            FrameType::Ok
        );
        assert_eq!(
            frame(
                &mut second,
                FrameType::JoinRoom,
                json!({"room_id":first_room})
            )
            .await
            .frame_type,
            FrameType::Error
        );
        assert_eq!(
            frame(
                &mut second,
                FrameType::SendMessage,
                json!({"room_id":first_room,"content":"wrong session"})
            )
            .await
            .frame_type,
            FrameType::Error
        );
        for index in 0..149 {
            assert_eq!(
                frame(
                    &mut first,
                    FrameType::SendMessage,
                    json!({"room_id":first_room,"content":format!("first-{index}")})
                )
                .await
                .frame_type,
                FrameType::Ok
            );
        }
        for index in 0..150 {
            let reply = frame(
                &mut second,
                FrameType::SendMessage,
                json!({"room_id":second_room,"content":format!("second-{index}")}),
            )
            .await;
            assert_eq!(
                reply.frame_type,
                if index < 50 {
                    FrameType::Ok
                } else {
                    FrameType::Error
                }
            );
        }
        let rooms = call(&base, owner_key, "GET", "rooms", json!({})).await;
        let rooms = rooms["rooms"].as_array().unwrap();
        assert!(rooms.iter().any(|room| room["room_id"] == first_room
            && room["name"] == "cowchat"
            && room["kind"] == "session"
            && room["session"]["agent_id"] == first_id));
        assert!(rooms.iter().any(|room| room["room_id"] == second_room
            && room["kind"] == "session"
            && room["session"]["agent_id"] == second_id));
        let mut more_sessions = Vec::new();
        for index in 0..28 {
            let id = format!("session:{}", uuid::Uuid::new_v4());
            let (mut socket, _) =
                tokio_tungstenite::connect_async(format!("{}/ws", base.replacen("http", "ws", 1)))
                    .await
                    .unwrap();
            let reply = frame(
                &mut socket,
                FrameType::Register,
                register(&id, &format!("worker-{index}"), &format!("branch-{index}")),
            )
            .await;
            assert_eq!(
                reply.frame_type,
                FrameType::Ok,
                "session {index}: {reply:?}"
            );
            more_sessions.push(socket);
        }
        assert_eq!(state.broker.agents.len(), 30);
        call(
            &base,
            first_token,
            "POST",
            &format!("sessions/{first_id}/heartbeat"),
            json!({}),
        )
        .await;
        call(
            &base,
            first_token,
            "POST",
            &format!("sessions/{first_id}/end"),
            json!({}),
        )
        .await;
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while state.broker.agents.contains_key(&first_id) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("ended session must disconnect");
        for credential in [bot_key, first_token] {
            let (mut resumed, _) =
                tokio_tungstenite::connect_async(format!("{}/ws", base.replacen("http", "ws", 1)))
                    .await
                    .unwrap();
            let mut attempt = register(&first_id, "claude-one", "main");
            attempt["key"] = json!(credential);
            assert_eq!(
                frame(&mut resumed, FrameType::Register, attempt)
                    .await
                    .frame_type,
                FrameType::Error
            );
        }
        assert_eq!(
            state.store.social_room_access(first_room, first_token),
            Some(false)
        );
        let rooms = call(&base, owner_key, "GET", "rooms", json!({})).await;
        assert!(rooms["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|room| room["room_id"] == first_room && room["session"]["ended_at"].is_number()));

        for task in tasks {
            task.abort();
        }
    }
}
