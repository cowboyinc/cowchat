use crate::config::CodexConfig;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{client_async, connect_async, WebSocketStream};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WakeReference {
    pub target: String,
    pub room: String,
    pub after_seq: i64,
    pub observed_seq: i64,
    pub source: String,
    pub event_id: String,
    pub event_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct CodexWakeOutcome {
    pub mode: String,
    pub prior_status: String,
    pub turn_id: String,
}

#[async_trait]
pub trait WakeBackend: Send + Sync {
    async fn wake(
        &self,
        thread_id: &str,
        reference: &WakeReference,
    ) -> Result<CodexWakeOutcome, AppServerError>;
}

#[derive(Clone)]
pub struct CodexAppServerClient {
    config: CodexConfig,
}

impl CodexAppServerClient {
    pub fn new(config: CodexConfig) -> Self {
        Self { config }
    }

    async fn connect_and_wake(
        &self,
        thread_id: &str,
        reference: &WakeReference,
    ) -> Result<CodexWakeOutcome, AppServerError> {
        let endpoint = &self.config.app_server_endpoint;
        if let Some(path) = endpoint.strip_prefix("unix://") {
            if path.is_empty() {
                return Err(AppServerError::InvalidEndpoint(
                    "unix endpoint requires an absolute socket path".into(),
                ));
            }
            let stream = UnixStream::connect(path)
                .await
                .map_err(AppServerError::UnixConnect)?;
            let request = self.websocket_request("ws://localhost/")?;
            let (websocket, _) = client_async(request, stream)
                .await
                .map_err(websocket_error)?;
            self.wake_over_websocket(websocket, thread_id, reference)
                .await
        } else if endpoint.starts_with("ws://") || endpoint.starts_with("wss://") {
            let request = self.websocket_request(endpoint)?;
            let (websocket, _) = connect_async(request).await.map_err(websocket_error)?;
            self.wake_over_websocket(websocket, thread_id, reference)
                .await
        } else {
            Err(AppServerError::InvalidEndpoint(endpoint.clone()))
        }
    }

    fn websocket_request(
        &self,
        url: &str,
    ) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, AppServerError> {
        let mut request = url.into_client_request().map_err(websocket_error)?;
        if let Some(env_name) = &self.config.bearer_token_env {
            let token = std::env::var(env_name)
                .map_err(|_| AppServerError::MissingToken(env_name.clone()))?;
            let value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| AppServerError::InvalidToken)?;
            request.headers_mut().insert("authorization", value);
        }
        Ok(request)
    }

    async fn wake_over_websocket<S>(
        &self,
        mut websocket: WebSocketStream<S>,
        thread_id: &str,
        reference: &WakeReference,
    ) -> Result<CodexWakeOutcome, AppServerError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let timeout = Duration::from_secs(self.config.request_timeout_seconds);
        send_request(
            &mut websocket,
            1,
            "initialize",
            json!({
                "clientInfo": {
                    "name": "cowchat_codex_wake",
                    "title": "Cowchat Codex Wake Bridge",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": true,
                    "optOutNotificationMethods": ["item/agentMessage/delta"]
                }
            }),
            timeout,
        )
        .await?;
        send_notification(&mut websocket, "initialized", json!({})).await?;

        let read = send_request(
            &mut websocket,
            2,
            "thread/read",
            json!({"threadId": thread_id, "includeTurns": false}),
            timeout,
        )
        .await?;
        let prior_status = read
            .pointer("/thread/status/type")
            .and_then(Value::as_str)
            .ok_or(AppServerError::MissingThreadStatus)?
            .to_string();
        if prior_status == "systemError" {
            return Err(AppServerError::ThreadSystemError(thread_id.to_string()));
        }
        if prior_status == "active"
            && read
                .pointer("/thread/canAcceptDirectInput")
                .and_then(Value::as_bool)
                == Some(false)
        {
            return Err(AppServerError::ActiveTurnNotSteerable(
                thread_id.to_string(),
            ));
        }
        if prior_status == "notLoaded" {
            send_request(
                &mut websocket,
                3,
                "thread/resume",
                json!({"threadId": thread_id}),
                timeout,
            )
            .await?;
        }

        let reference_json = serde_json::to_string(reference)?;
        let turn = send_request(
            &mut websocket,
            4,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [],
                "additionalContext": {
                    "cowchat_wake": {
                        "kind": "untrusted",
                        "value": reference_json
                    }
                },
                "responsesapiClientMetadata": {
                    "cowchat_wake_source": reference.source,
                    "cowchat_wake_event_id": reference.event_id
                }
            }),
            timeout,
        )
        .await?;
        let turn_id = turn
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .ok_or(AppServerError::MissingTurnId)?
            .to_string();
        Ok(CodexWakeOutcome {
            mode: if prior_status == "active" {
                "steered".to_string()
            } else {
                "started".to_string()
            },
            prior_status,
            turn_id,
        })
    }
}

#[async_trait]
impl WakeBackend for CodexAppServerClient {
    async fn wake(
        &self,
        thread_id: &str,
        reference: &WakeReference,
    ) -> Result<CodexWakeOutcome, AppServerError> {
        self.connect_and_wake(thread_id, reference).await
    }
}

async fn send_notification<S>(
    websocket: &mut WebSocketStream<S>,
    method: &str,
    params: Value,
) -> Result<(), AppServerError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    websocket
        .send(Message::Text(
            json!({"method": method, "params": params})
                .to_string()
                .into(),
        ))
        .await
        .map_err(websocket_error)
}

async fn send_request<S>(
    websocket: &mut WebSocketStream<S>,
    id: i64,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, AppServerError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    websocket
        .send(Message::Text(
            json!({"method": method, "id": id, "params": params})
                .to_string()
                .into(),
        ))
        .await
        .map_err(websocket_error)?;

    tokio::time::timeout(timeout, async {
        loop {
            let message = websocket
                .next()
                .await
                .ok_or(AppServerError::ConnectionClosed)?
                .map_err(websocket_error)?;
            let text = match message {
                Message::Text(text) => text.to_string(),
                Message::Binary(bytes) => String::from_utf8(bytes.to_vec()).map_err(|_| {
                    AppServerError::InvalidJson("binary response is not UTF-8".into())
                })?,
                Message::Close(_) => return Err(AppServerError::ConnectionClosed),
                _ => continue,
            };
            let response: Value = serde_json::from_str(&text)
                .map_err(|error| AppServerError::InvalidJson(error.to_string()))?;
            if response.get("id").and_then(Value::as_i64) != Some(id) {
                continue;
            }
            if let Some(error) = response.get("error") {
                return Err(AppServerError::Rpc(error.to_string()));
            }
            return response
                .get("result")
                .cloned()
                .ok_or_else(|| AppServerError::InvalidJson("response has no result".into()));
        }
    })
    .await
    .map_err(|_| AppServerError::Timeout(method.to_string()))?
}

#[derive(Debug, thiserror::Error)]
pub enum AppServerError {
    #[error(
        "invalid Codex app-server endpoint {0:?}; use ws://, wss://, or unix:///absolute/path"
    )]
    InvalidEndpoint(String),
    #[error("failed to connect to Codex app-server Unix socket: {0}")]
    UnixConnect(#[source] std::io::Error),
    #[error("Codex app-server WebSocket error: {0}")]
    WebSocket(#[source] Box<tokio_tungstenite::tungstenite::Error>),
    #[error("environment variable {0} is required for the Codex app-server bearer token")]
    MissingToken(String),
    #[error("Codex app-server bearer token is not a valid HTTP header value")]
    InvalidToken,
    #[error("Codex app-server connection closed before the response arrived")]
    ConnectionClosed,
    #[error("Codex app-server request {0} timed out")]
    Timeout(String),
    #[error("invalid Codex app-server JSON: {0}")]
    InvalidJson(String),
    #[error("Codex app-server returned an error: {0}")]
    Rpc(String),
    #[error("Codex app-server thread/read response omitted runtime status")]
    MissingThreadStatus,
    #[error("Codex thread {0} is in systemError state")]
    ThreadSystemError(String),
    #[error("Codex thread {0} has an active review or compaction turn that cannot accept a wake")]
    ActiveTurnNotSteerable(String),
    #[error("Codex app-server turn/start response omitted turn id")]
    MissingTurnId,
    #[error("failed to serialize wake reference: {0}")]
    Serialize(#[from] serde_json::Error),
}

fn websocket_error(error: tokio_tungstenite::tungstenite::Error) -> AppServerError {
    AppServerError::WebSocket(Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

    #[tokio::test]
    async fn wakes_with_empty_input_and_untrusted_thin_reference() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            let mut methods = Vec::new();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let request: Value = serde_json::from_str(&text).unwrap();
                let method = request["method"].as_str().unwrap().to_string();
                methods.push(method.clone());
                if method == "initialized" {
                    continue;
                }
                let id = request["id"].as_i64().unwrap();
                let result = match method.as_str() {
                    "initialize" => json!({"userAgent": "test"}),
                    "thread/read" => json!({
                        "thread": {"id": "thr-1", "status": {"type": "notLoaded"}}
                    }),
                    "thread/resume" => json!({
                        "thread": {"id": "thr-1", "status": {"type": "idle"}}
                    }),
                    "turn/start" => {
                        assert_eq!(request["params"]["input"], json!([]));
                        assert_eq!(
                            request["params"]["additionalContext"]["cowchat_wake"]["kind"],
                            "untrusted"
                        );
                        let value = request["params"]["additionalContext"]["cowchat_wake"]["value"]
                            .as_str()
                            .unwrap();
                        assert!(!value.contains("payload"));
                        json!({"turn": {"id": "turn-1", "status": "inProgress"}})
                    }
                    _ => panic!("unexpected method {method}"),
                };
                ws.send(Message::Text(
                    json!({"id": id, "result": result}).to_string().into(),
                ))
                .await
                .unwrap();
                if method == "turn/start" {
                    break;
                }
            }
            methods
        });

        let client = CodexAppServerClient::new(CodexConfig {
            app_server_endpoint: format!("ws://{address}"),
            bearer_token_env: None,
            request_timeout_seconds: 2,
            wake_lease_seconds: 30,
        });
        let outcome = client
            .wake(
                "thr-1",
                &WakeReference {
                    target: "reviewer".into(),
                    room: "room".into(),
                    after_seq: 3,
                    observed_seq: 4,
                    source: "ci".into(),
                    event_id: "evt-1".into(),
                    event_type: "build.completed".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(outcome.mode, "started");
        assert_eq!(outcome.turn_id, "turn-1");
        assert_eq!(
            server.await.unwrap(),
            vec![
                "initialize",
                "initialized",
                "thread/read",
                "thread/resume",
                "turn/start"
            ]
        );
    }

    #[tokio::test]
    async fn refuses_active_turn_that_cannot_accept_direct_input() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let request: Value = serde_json::from_str(&text).unwrap();
                let method = request["method"].as_str().unwrap();
                if method == "initialized" {
                    continue;
                }
                let id = request["id"].as_i64().unwrap();
                let result = match method {
                    "initialize" => json!({"userAgent": "test"}),
                    "thread/read" => json!({
                        "thread": {
                            "id": "thr-1",
                            "status": {"type": "active"},
                            "canAcceptDirectInput": false
                        }
                    }),
                    _ => panic!("unexpected method {method}"),
                };
                ws.send(Message::Text(
                    json!({"id": id, "result": result}).to_string().into(),
                ))
                .await
                .unwrap();
                if method == "thread/read" {
                    break;
                }
            }
        });

        let client = CodexAppServerClient::new(CodexConfig {
            app_server_endpoint: format!("ws://{address}"),
            bearer_token_env: None,
            request_timeout_seconds: 2,
            wake_lease_seconds: 30,
        });
        let result = client
            .wake(
                "thr-1",
                &WakeReference {
                    target: "reviewer".into(),
                    room: "room".into(),
                    after_seq: 3,
                    observed_seq: 4,
                    source: "ci".into(),
                    event_id: "evt-1".into(),
                    event_type: "build.completed".into(),
                },
            )
            .await;
        assert!(matches!(
            result,
            Err(AppServerError::ActiveTurnNotSteerable(_))
        ));
        server.await.unwrap();
    }
}
