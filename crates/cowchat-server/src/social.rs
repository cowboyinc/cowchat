use crate::web::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

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
        .route("/social/v1/capabilities", get(capabilities))
        .route(
            "/social/v1/{*path}",
            get(action).post(action).patch(action).delete(action),
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
    Json(json!({"providers":providers,"cowboy_login":false}))
}
async fn action(
    State(state): State<AppState>,
    Path(path): Path<String>,
    headers: HeaderMap,
    method: Method,
    body: Option<Json<Value>>,
) -> Response {
    if configuration(&state).is_none() {
        return disabled();
    }
    let Some(key) = crate::web::authenticated_key(&state, &headers) else {
        return fail(anyhow::anyhow!("unauthorized"));
    };
    match state.store.social_action(
        &key,
        method.as_str(),
        &path,
        &body.map(|b| b.0).unwrap_or(json!({})),
    ) {
        Ok(value) => {
            if method != Method::GET {
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
    async fn call(base: &str, key: &str, method: &str, path: &str, body: Value) -> Value {
        let response = reqwest::Client::new()
            .request(method.parse().unwrap(), format!("{base}/social/v1/{path}"))
            .header("x-cowchat-key", key)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let value: Value = response.json().await.unwrap();
        assert_eq!(status, StatusCode::OK, "{path}: {value}");
        value
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
}
