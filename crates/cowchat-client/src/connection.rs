use cowchat_core::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpStream, UnixStream};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

const MAX_FRAME_BYTES: usize = 1024 * 1024;

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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn client_answers_server_heartbeat_ping() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read_half, mut write_half) = tokio::io::split(stream);
            let mut reader = BufReader::new(read_half);
            let register = read_frame_line(&mut reader).await.unwrap().unwrap();
            let register = Frame::from_line(&register).unwrap();
            let ok = Frame::ok(
                register.id.as_deref(),
                serde_json::json!({"agent_id": "heartbeat-client"}),
            );
            write_half
                .write_all(ok.to_line().unwrap().as_bytes())
                .await
                .unwrap();

            let ping = Frame {
                id: Some("heartbeat-1".into()),
                reply_to: None,
                frame_type: FrameType::Ping,
                payload: serde_json::json!({"heartbeat": true}),
            };
            write_half
                .write_all(ping.to_line().unwrap().as_bytes())
                .await
                .unwrap();
            let pong = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                read_frame_line(&mut reader),
            )
            .await
            .unwrap()
            .unwrap()
            .unwrap();
            let pong = Frame::from_line(&pong).unwrap();
            assert_eq!(pong.frame_type, FrameType::Pong);
            assert_eq!(pong.reply_to.as_deref(), Some("heartbeat-1"));
        });

        let _client = CowchatClient::connect_tcp(
            &addr.to_string(),
            "test-key",
            "client",
            Some("heartbeat-client"),
            vec![],
        )
        .await
        .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn v2_client_is_rejected_promptly_by_v1_server_semantics() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read_half, mut write_half) = tokio::io::split(stream);
            let mut reader = BufReader::new(read_half);
            let register = read_frame_line(&mut reader).await.unwrap().unwrap();
            let register = Frame::from_line(&register).unwrap();
            assert_eq!(
                register.payload["protocol_version"],
                serde_json::json!(2),
                "the current client must negotiate protocol v2"
            );
            let error = Frame::error(
                register.id.as_deref(),
                ErrorPayload::new(
                    ErrorCode::UnsupportedProtocol,
                    "Client protocol v2 is newer than this server (v1); the server needs an upgrade",
                ),
            );
            write_half
                .write_all(error.to_line().unwrap().as_bytes())
                .await
                .unwrap();
        });

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            CowchatClient::connect_tcp(&addr.to_string(), "test-key", "v2-client", None, vec![]),
        )
        .await
        .expect("protocol mismatch must fail during registration");
        match result {
            Err(ClientError::Server { code, .. }) => {
                assert_eq!(code, ErrorCode::UnsupportedProtocol)
            }
            Ok(_) => panic!("a v1 server must reject a v2 client"),
            Err(other) => panic!("expected UnsupportedProtocol, got {other:?}"),
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn subscribers_are_closed_when_the_transport_reaches_eof() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read_half, mut write_half) = tokio::io::split(stream);
            let mut reader = BufReader::new(read_half);
            let register = read_frame_line(&mut reader).await.unwrap().unwrap();
            let register = Frame::from_line(&register).unwrap();
            let ok = Frame::ok(
                register.id.as_deref(),
                serde_json::json!({"agent_id": "eof-client"}),
            );
            write_half
                .write_all(ok.to_line().unwrap().as_bytes())
                .await
                .unwrap();
            write_half.shutdown().await.unwrap();
        });

        let client = CowchatClient::connect_tcp(
            &addr.to_string(),
            "test-key",
            "client",
            Some("eof-client"),
            vec![],
        )
        .await
        .unwrap();
        let mut events = client.subscribe();
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("transport EOF must close subscribers promptly");
        assert!(matches!(result, Err(broadcast::error::RecvError::Closed)));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn contiguous_history_rejects_a_missing_first_sequence() {
        let (write_tx, mut write_rx) = mpsc::channel(1);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, _) = broadcast::channel(1);
        let client = CowchatClient {
            write_tx,
            pending: pending.clone(),
            event_tx: event_tx.downgrade(),
            agent_id: "reader".into(),
            agent_name: "reader".into(),
            stable_identity: true,
            room_secret: None,
        };
        let responder = tokio::spawn(async move {
            let request = write_rx.recv().await.unwrap();
            assert_eq!(request.frame_type, FrameType::GetHistory);
            let message = ChatMessage {
                message_id: "second".into(),
                room_id: "lobby".into(),
                agent_id: "peer".into(),
                agent_name: "peer".into(),
                content: "seq one was retained away".into(),
                reply_to_message: None,
                metadata: serde_json::json!({}),
                timestamp: chrono::Utc::now(),
                seq: 2,
            };
            let response = Frame {
                id: Some("history-response".into()),
                reply_to: request.id.clone(),
                frame_type: FrameType::HistoryResult,
                payload: serde_json::json!({ "messages": [message] }),
            };
            pending
                .lock()
                .await
                .remove(request.id.as_deref().unwrap())
                .unwrap()
                .send(response)
                .unwrap();
        });

        let error = client
            .get_contiguous_history_page("lobby", 0, 2, 50)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ClientError::HistoryGap {
                expected: 1,
                found: Some(2),
                ..
            }
        ));
        responder.await.unwrap();
    }

    #[tokio::test]
    async fn wait_rejects_a_cursor_ahead_of_the_captured_tip() {
        let (write_tx, mut write_rx) = mpsc::channel(1);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, _) = broadcast::channel(1);
        let client = CowchatClient {
            write_tx,
            pending: pending.clone(),
            event_tx: event_tx.downgrade(),
            agent_id: "reader".into(),
            agent_name: "reader".into(),
            stable_identity: true,
            room_secret: None,
        };
        let responder = tokio::spawn(async move {
            let request = write_rx.recv().await.unwrap();
            assert_eq!(request.frame_type, FrameType::RoomTip);
            pending
                .lock()
                .await
                .remove(request.id.as_deref().unwrap())
                .unwrap()
                .send(Frame {
                    id: Some("tip-response".into()),
                    reply_to: request.id,
                    frame_type: FrameType::RoomTipResult,
                    payload: serde_json::json!({ "room_id": "lobby", "seq": 4 }),
                })
                .unwrap();
        });

        let error = client
            .wait_for_message("lobby", 60, Some(5))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ClientError::HistoryCursorAhead {
                after_seq: 5,
                through_seq: 4,
                ..
            }
        ));
        responder.await.unwrap();
    }

    #[tokio::test]
    async fn live_sequence_jump_requires_contiguous_history_recovery() {
        let (write_tx, mut write_rx) = mpsc::channel(1);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, _) = broadcast::channel(4);
        let client = CowchatClient {
            write_tx,
            pending: pending.clone(),
            event_tx: event_tx.downgrade(),
            agent_id: "reader".into(),
            agent_name: "reader".into(),
            stable_identity: true,
            room_secret: None,
        };
        let mut events = client.subscribe();
        event_tx
            .send(Event {
                frame: Frame {
                    id: None,
                    reply_to: None,
                    frame_type: FrameType::MessageReceived,
                    payload: serde_json::to_value(ChatMessage {
                        message_id: "second".into(),
                        room_id: "lobby".into(),
                        agent_id: "peer".into(),
                        agent_name: "peer".into(),
                        content: "jumped live event".into(),
                        reply_to_message: None,
                        metadata: serde_json::json!({}),
                        timestamp: chrono::Utc::now(),
                        seq: 2,
                    })
                    .unwrap(),
                },
            })
            .unwrap();

        let responder = tokio::spawn(async move {
            let tip_request = write_rx.recv().await.unwrap();
            assert_eq!(tip_request.frame_type, FrameType::RoomTip);
            pending
                .lock()
                .await
                .remove(tip_request.id.as_deref().unwrap())
                .unwrap()
                .send(Frame {
                    id: Some("tip-response".into()),
                    reply_to: tip_request.id,
                    frame_type: FrameType::RoomTipResult,
                    payload: serde_json::json!({ "seq": 2 }),
                })
                .unwrap();

            let history_request = write_rx.recv().await.unwrap();
            assert_eq!(history_request.frame_type, FrameType::GetHistory);
            pending
                .lock()
                .await
                .remove(history_request.id.as_deref().unwrap())
                .unwrap()
                .send(Frame {
                    id: Some("history-response".into()),
                    reply_to: history_request.id,
                    frame_type: FrameType::HistoryResult,
                    payload: serde_json::json!({
                        "messages": [{
                            "message_id": "second",
                            "room_id": "lobby",
                            "agent_id": "peer",
                            "agent_name": "peer",
                            "content": "jumped persisted event",
                            "reply_to_message": null,
                            "metadata": {},
                            "timestamp": chrono::Utc::now(),
                            "seq": 2
                        }]
                    }),
                })
                .unwrap();
        });

        let error = client
            .wait_for_live_message("lobby", 1, Some(0), &mut events)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ClientError::HistoryGap {
                expected: 1,
                found: Some(2),
                ..
            }
        ));
        responder.await.unwrap();
    }

    #[tokio::test]
    async fn lagged_wait_fails_closed_instead_of_returning_a_later_message() {
        let (write_tx, _write_rx) = mpsc::channel(1);
        let (event_tx, _) = broadcast::channel(256);
        let client = CowchatClient {
            write_tx,
            pending: Arc::new(Mutex::new(HashMap::new())),
            event_tx: event_tx.downgrade(),
            agent_id: "stable-waiter".into(),
            agent_name: "waiter".into(),
            stable_identity: true,
            room_secret: None,
        };
        let mut events = client.subscribe();

        for seq in 1..=300 {
            event_tx
                .send(Event {
                    frame: Frame {
                        id: None,
                        reply_to: None,
                        frame_type: FrameType::MessageReceived,
                        payload: serde_json::json!({
                            "room_id": "lobby",
                            "seq": seq,
                        }),
                    },
                })
                .unwrap();
        }

        let error = client
            .wait_for_live_message("lobby", 1, None, &mut events)
            .await
            .expect_err("a lag without an authoritative history floor must fail closed");
        assert!(matches!(
            error,
            ClientError::EventStreamLagged { skipped } if skipped >= 44
        ));
    }

    #[tokio::test]
    async fn lagged_wait_backfills_from_original_cursor_before_returning_live_tail() {
        let (write_tx, mut write_rx) = mpsc::channel(1);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, _) = broadcast::channel(256);
        let client = CowchatClient {
            write_tx,
            pending: pending.clone(),
            event_tx: event_tx.downgrade(),
            agent_id: "stable-waiter".into(),
            agent_name: "waiter".into(),
            stable_identity: true,
            room_secret: None,
        };
        let mut events = client.subscribe();
        for seq in 101..=400 {
            event_tx
                .send(Event {
                    frame: Frame {
                        id: None,
                        reply_to: None,
                        frame_type: FrameType::MessageReceived,
                        payload: serde_json::json!({ "room_id": "lobby", "seq": seq }),
                    },
                })
                .unwrap();
        }

        let responder = tokio::spawn(async move {
            let request = write_rx.recv().await.unwrap();
            assert_eq!(request.frame_type, FrameType::RoomTip);
            let response = Frame {
                id: Some("tip-response".into()),
                reply_to: request.id.clone(),
                frame_type: FrameType::RoomTipResult,
                payload: serde_json::json!({ "room_id": "lobby", "seq": 1 }),
            };
            pending
                .lock()
                .await
                .remove(request.id.as_deref().unwrap())
                .unwrap()
                .send(response)
                .unwrap();

            let request = write_rx.recv().await.unwrap();
            assert_eq!(request.frame_type, FrameType::GetHistory);
            assert_eq!(request.payload["since_seq"], serde_json::json!(0));
            let earliest = ChatMessage {
                message_id: "earliest".into(),
                room_id: "lobby".into(),
                agent_id: "peer".into(),
                agent_name: "peer".into(),
                content: "persisted before the retained tail".into(),
                reply_to_message: None,
                metadata: serde_json::json!({}),
                timestamp: chrono::Utc::now(),
                seq: 1,
            };
            let response = Frame {
                id: Some("history-response".into()),
                reply_to: request.id.clone(),
                frame_type: FrameType::HistoryResult,
                payload: serde_json::json!({ "messages": [earliest] }),
            };
            pending
                .lock()
                .await
                .remove(request.id.as_deref().unwrap())
                .unwrap()
                .send(response)
                .unwrap();
        });

        let message = client
            .wait_for_live_message("lobby", 1, Some(0), &mut events)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(message.seq, 1);
        assert_eq!(message.content, "persisted before the retained tail");
        responder.await.unwrap();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Server error: {code:?} - {message}")]
    Server { code: ErrorCode, message: String },
    #[error("Connection closed")]
    ConnectionClosed,
    #[error("Request timed out")]
    Timeout,
    #[error("Channel error")]
    Channel,
    #[error("WebSocket error: {0}")]
    Ws(String),
    #[error("Event stream lagged by {skipped} frame(s); history backfill could not recover contiguously")]
    EventStreamLagged { skipped: u64 },
    #[error(
        "History gap in room {room_id}: expected seq {expected}, found {found:?}; refusing to skip unread events"
    )]
    HistoryGap {
        room_id: String,
        expected: i64,
        found: Option<i64>,
    },
    #[error(
        "History cursor for room {room_id} is ahead of captured tip ({after_seq} > {through_seq})"
    )]
    HistoryCursorAhead {
        room_id: String,
        after_seq: i64,
        through_seq: i64,
    },
}

/// An event received from the server (pushed, not in response to a request).
#[derive(Debug, Clone)]
pub struct Event {
    pub frame: Frame,
}

pub struct CowchatClient {
    /// Channel to send frames to the writer task.
    write_tx: mpsc::Sender<Frame>,
    /// Pending request completions: correlation_id -> oneshot sender.
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Frame>>>>,
    /// Broadcast channel for server-pushed events.
    event_tx: broadcast::WeakSender<Event>,
    /// Agent info after registration.
    pub agent_id: String,
    pub agent_name: String,
    stable_identity: bool,
    /// Pre-shared secret for end-to-end encrypted rooms. When set, message
    /// `content` is encrypted before send and decrypted after receive, keyed
    /// per-room. None means the client sends/receives plaintext.
    room_secret: Option<Vec<u8>>,
}

impl CowchatClient {
    /// Connect via Unix domain socket and register.
    pub async fn connect_uds(
        socket_path: &Path,
        key: &str,
        name: &str,
        agent_id: Option<&str>,
        capabilities: Vec<String>,
    ) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket_path).await?;
        let (read_half, write_half) = tokio::io::split(stream);
        Self::setup(read_half, write_half, key, name, agent_id, capabilities).await
    }

    /// Connect via TCP and register.
    pub async fn connect_tcp(
        addr: &str,
        key: &str,
        name: &str,
        agent_id: Option<&str>,
        capabilities: Vec<String>,
    ) -> Result<Self, ClientError> {
        let stream = TcpStream::connect(addr).await?;
        let (read_half, write_half) = tokio::io::split(stream);
        Self::setup(read_half, write_half, key, name, agent_id, capabilities).await
    }

    async fn setup<R, W>(
        read_half: R,
        write_half: W,
        key: &str,
        name: &str,
        agent_id: Option<&str>,
        capabilities: Vec<String>,
    ) -> Result<Self, ClientError>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (write_tx, mut write_rx) = mpsc::channel::<Frame>(256);
        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Frame>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, _) = broadcast::channel::<Event>(256);

        // Writer task
        let mut write_half = write_half;
        tokio::spawn(async move {
            while let Some(frame) = write_rx.recv().await {
                match frame.to_line() {
                    Ok(line) => {
                        if write_half.write_all(line.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        log::error!("Client frame serialization error: {}", e);
                    }
                }
            }
            // Channel closed (client dropped or shutting down) — explicitly close the
            // write side so the server detects EOF promptly instead of waiting for the
            // heartbeat timeout. `tokio::io::split` keeps the TcpStream alive while the
            // read half is held by the reader task, so just dropping write_half is not
            // enough to send FIN.
            let _ = write_half.shutdown().await;
        });

        // Reader task
        let pending_clone = pending.clone();
        let event_tx_clone = event_tx.clone();
        // A weak sender lets dropping the client close the writer even while
        // the reader task is still blocked on the socket.
        let pong_tx = write_tx.downgrade();
        tokio::spawn(async move {
            let mut reader = BufReader::new(read_half);
            loop {
                match read_frame_line(&mut reader).await {
                    Ok(None) => break, // EOF
                    Ok(Some(line)) => {
                        if let Ok(frame) = Frame::from_line(&line) {
                            if frame.frame_type == FrameType::Ping {
                                if let Some(tx) = pong_tx.upgrade() {
                                    let _ = tx.send(Frame::pong(frame.id.as_deref())).await;
                                }
                                continue;
                            }
                            // Check if this is a response to a pending request
                            if let Some(reply_to) = &frame.reply_to {
                                let mut pending = pending_clone.lock().await;
                                if let Some(sender) = pending.remove(reply_to) {
                                    let _ = sender.send(frame);
                                    continue;
                                }
                            }
                            // Otherwise it's a pushed event
                            let _ = event_tx_clone.send(Event { frame });
                        }
                    }
                    Err(_) => break,
                }
            }
            // Wake in-flight requests immediately, then let this task's sole
            // strong event sender drop so subscribers observe transport close.
            pending_clone.lock().await.clear();
        });

        Self::finish_register(
            write_tx,
            pending,
            event_tx,
            key,
            name,
            agent_id,
            capabilities,
        )
        .await
    }

    /// Connect via WebSocket (ws:// or wss://) and register. The URL should
    /// point at the server's `/ws` endpoint, e.g. `wss://your-server.example/ws`.
    /// Speaks the same NDJSON protocol, one frame per WebSocket text message.
    pub async fn connect_ws(
        url: &str,
        key: &str,
        name: &str,
        agent_id: Option<&str>,
        capabilities: Vec<String>,
    ) -> Result<Self, ClientError> {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        // rustls 0.23 needs an explicit process-wide crypto provider. Install
        // ring (idempotent; errors if already set, which we ignore).
        let _ = rustls::crypto::ring::default_provider().install_default();

        let (ws, _resp) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| ClientError::Ws(e.to_string()))?;
        let (mut sink, mut stream) = ws.split();

        let (write_tx, mut write_rx) = mpsc::channel::<Frame>(256);
        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Frame>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, _) = broadcast::channel::<Event>(256);

        // Writer task: each frame is sent as one WS text message (NDJSON line).
        tokio::spawn(async move {
            while let Some(frame) = write_rx.recv().await {
                match frame.to_line() {
                    Ok(line) => {
                        if sink.send(Message::Text(line.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => log::error!("Client frame serialization error: {}", e),
                }
            }
            let _ = sink.close().await;
        });

        // Reader task: each WS text message is one frame.
        let pending_clone = pending.clone();
        let event_tx_clone = event_tx.clone();
        let pong_tx = write_tx.downgrade();
        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                let text = match msg {
                    Ok(Message::Text(t)) => t.as_str().to_string(),
                    Ok(Message::Binary(b)) => String::from_utf8_lossy(&b).into_owned(),
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(_) => continue, // ping/pong/frame — ignore
                };
                if text.len() > MAX_FRAME_BYTES {
                    break;
                }
                if let Ok(frame) = Frame::from_line(&text) {
                    if frame.frame_type == FrameType::Ping {
                        if let Some(tx) = pong_tx.upgrade() {
                            let _ = tx.send(Frame::pong(frame.id.as_deref())).await;
                        }
                        continue;
                    }
                    if let Some(reply_to) = &frame.reply_to {
                        let mut pending = pending_clone.lock().await;
                        if let Some(sender) = pending.remove(reply_to) {
                            let _ = sender.send(frame);
                            continue;
                        }
                    }
                    let _ = event_tx_clone.send(Event { frame });
                }
            }
            pending_clone.lock().await.clear();
        });

        Self::finish_register(
            write_tx,
            pending,
            event_tx,
            key,
            name,
            agent_id,
            capabilities,
        )
        .await
    }

    /// Shared post-transport setup: perform the register handshake over the
    /// already-wired channels and construct the client. Used by every transport
    /// (UDS, TCP, WebSocket).
    async fn finish_register(
        write_tx: mpsc::Sender<Frame>,
        pending: Arc<Mutex<HashMap<String, oneshot::Sender<Frame>>>>,
        event_tx: broadcast::Sender<Event>,
        key: &str,
        name: &str,
        agent_id: Option<&str>,
        capabilities: Vec<String>,
    ) -> Result<Self, ClientError> {
        let stable_identity = agent_id.is_some();
        let register_frame = Frame {
            id: Some(uuid::Uuid::new_v4().to_string()),
            reply_to: None,
            frame_type: FrameType::Register,
            payload: serde_json::to_value(RegisterPayload {
                key: key.to_string(),
                agent_id: agent_id.map(String::from),
                name: name.to_string(),
                capabilities,
                // A caller supplying a stable agent_id intends to resume that
                // identity across one-shot calls, so register as a reconnect —
                // otherwise the second call collides with the id still held open
                // during the previous connection's reconnect window.
                reconnect: agent_id.is_some(),
                protocol_version: Some(cowchat_core::PROTOCOL_VERSION),
            })
            .unwrap(),
        };

        let req_id = register_frame.id.clone().unwrap();
        let (resp_tx, resp_rx) = oneshot::channel();
        {
            let mut p = pending.lock().await;
            p.insert(req_id.clone(), resp_tx);
        }
        write_tx
            .send(register_frame)
            .await
            .map_err(|_| ClientError::Channel)?;

        let response = tokio::time::timeout(std::time::Duration::from_secs(5), resp_rx)
            .await
            .map_err(|_| ClientError::Timeout)?
            .map_err(|_| ClientError::ConnectionClosed)?;

        if response.frame_type == FrameType::Error {
            let err: ErrorPayload = serde_json::from_value(response.payload)
                .unwrap_or(ErrorPayload::new(ErrorCode::InternalError, "Unknown error"));
            return Err(ClientError::Server {
                code: err.code,
                message: err.message,
            });
        }

        let agent_id = response
            .payload
            .get("agent_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        Ok(Self {
            write_tx,
            pending,
            event_tx: event_tx.downgrade(),
            agent_id,
            agent_name: name.to_string(),
            stable_identity,
            room_secret: None,
        })
    }

    /// Configure the pre-shared secret for end-to-end encrypted rooms. With a
    /// secret set, `send_message`/`thinking`/`send_decision` encrypt `content`
    /// before sending, and `get_history`/`wait_for_message` decrypt it after
    /// receiving — both keyed per-room. Set this before sending or receiving.
    pub fn set_room_secret(&mut self, secret: &[u8]) {
        self.room_secret = Some(secret.to_vec());
    }

    /// Encrypt `content` for `room_id` if a room secret is configured, otherwise
    /// return it unchanged.
    fn encrypt_content(&self, room_id: &str, content: &str) -> String {
        match &self.room_secret {
            Some(secret) => cowchat_core::crypto::encrypt(secret, room_id, content),
            None => content.to_string(),
        }
    }

    /// Decrypt `msg.content` in place when a room secret is set and the content
    /// is a Cowchat ciphertext blob. Leaves content untouched on decrypt
    /// failure or when no key is configured (callers then see the `cow1:` blob).
    fn decrypt_message(&self, msg: &mut ChatMessage) {
        if let Some(secret) = &self.room_secret {
            if cowchat_core::crypto::is_ciphertext(&msg.content) {
                if let Ok(plain) = cowchat_core::crypto::decrypt(secret, &msg.room_id, &msg.content)
                {
                    msg.content = plain;
                }
            }
        }
    }

    /// Whether a message was posted by this logical client. With a stable
    /// agent_id, identity is ID-only so two distinct agents may safely share a
    /// display name. Name fallback is retained only for legacy one-shot calls.
    pub fn is_self_message(&self, msg: &ChatMessage) -> bool {
        msg.agent_id == self.agent_id
            || (!self.stable_identity && msg.agent_name == self.agent_name)
    }

    /// Send a request and wait for the response.
    async fn request(
        &self,
        frame_type: FrameType,
        payload: serde_json::Value,
    ) -> Result<Frame, ClientError> {
        let id = uuid::Uuid::new_v4().to_string();
        let frame = Frame {
            id: Some(id.clone()),
            reply_to: None,
            frame_type,
            payload,
        };

        let (resp_tx, resp_rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id.clone(), resp_tx);
        }

        match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.write_tx.send(frame),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                self.pending.lock().await.remove(&id);
                return Err(ClientError::Channel);
            }
            Err(_) => {
                self.pending.lock().await.remove(&id);
                return Err(ClientError::Timeout);
            }
        }

        let response = match tokio::time::timeout(std::time::Duration::from_secs(10), resp_rx).await
        {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => {
                self.pending.lock().await.remove(&id);
                return Err(ClientError::ConnectionClosed);
            }
            Err(_) => {
                self.pending.lock().await.remove(&id);
                return Err(ClientError::Timeout);
            }
        };

        if response.frame_type == FrameType::Error {
            let err: ErrorPayload = serde_json::from_value(response.payload)
                .unwrap_or(ErrorPayload::new(ErrorCode::InternalError, "Unknown error"));
            return Err(ClientError::Server {
                code: err.code,
                message: err.message,
            });
        }

        Ok(response)
    }

    // --- High-level API ---

    pub async fn ping(&self) -> Result<(), ClientError> {
        self.request(FrameType::Ping, serde_json::json!({})).await?;
        Ok(())
    }

    pub async fn create_room(
        &self,
        name: &str,
        description: Option<&str>,
        parent_id: Option<&str>,
        ephemeral: bool,
    ) -> Result<Room, ClientError> {
        self.create_room_with_options(name, description, parent_id, ephemeral, false, false)
            .await
    }

    /// Like `create_room` but lets you mark the room `public` (visible/joinable
    /// by any API key) and/or end-to-end `encrypted` (server stores only
    /// ciphertext; members must share a room secret — see `set_room_secret`).
    #[allow(clippy::too_many_arguments)]
    pub async fn create_room_with_options(
        &self,
        name: &str,
        description: Option<&str>,
        parent_id: Option<&str>,
        ephemeral: bool,
        public: bool,
        encrypted: bool,
    ) -> Result<Room, ClientError> {
        let resp = self
            .request(
                FrameType::CreateRoom,
                serde_json::to_value(CreateRoomPayload {
                    name: name.to_string(),
                    description: description.map(String::from),
                    parent_id: parent_id.map(String::from),
                    ephemeral,
                    public,
                    encrypted,
                })
                .unwrap(),
            )
            .await?;
        Ok(serde_json::from_value(resp.payload).unwrap())
    }

    pub async fn join_room(&self, room_id: &str) -> Result<(), ClientError> {
        self.request(FrameType::JoinRoom, serde_json::json!({"room_id": room_id}))
            .await?;
        Ok(())
    }

    pub async fn leave_room(&self, room_id: &str) -> Result<(), ClientError> {
        self.request(
            FrameType::LeaveRoom,
            serde_json::json!({"room_id": room_id}),
        )
        .await?;
        Ok(())
    }

    /// Rename a room created by this exact registered agent.
    ///
    /// The server trims and validates `name`, rejects collisions, and returns
    /// the complete updated room object.
    pub async fn rename_room(&self, room_id: &str, name: &str) -> Result<Room, ClientError> {
        let response = self
            .request(
                FrameType::RenameRoom,
                serde_json::to_value(RenameRoomPayload {
                    room_id: room_id.to_string(),
                    name: name.to_string(),
                })
                .unwrap(),
            )
            .await?;
        Ok(serde_json::from_value(response.payload).unwrap())
    }

    /// Irreversibly remove a room from Cowchat's active application state.
    /// The server rejects system rooms and callers registered under a different
    /// agent ID. This does not promise forensic erasure from storage snapshots
    /// or backups. Room UUIDs are lifecycle tombstones and are never reused.
    pub async fn destroy_room(&self, room_id: &str) -> Result<(), ClientError> {
        self.request(
            FrameType::DestroyRoom,
            serde_json::to_value(DestroyRoomPayload {
                room_id: room_id.to_string(),
            })
            .unwrap(),
        )
        .await?;
        Ok(())
    }

    pub async fn send_message(
        &self,
        room_id: &str,
        content: &str,
        reply_to: Option<&str>,
        mentions: Vec<String>,
    ) -> Result<ChatMessage, ClientError> {
        self.send_message_with_metadata(room_id, content, reply_to, mentions, serde_json::json!({}))
            .await
    }

    /// Like `send_message` but lets you attach arbitrary metadata. Used by
    /// `cowchat send --kind X` to tag messages with `metadata.kind = X` so
    /// peers can filter on it via `wait --only-kind` / `history --kind`.
    pub async fn send_message_with_metadata(
        &self,
        room_id: &str,
        content: &str,
        reply_to: Option<&str>,
        mentions: Vec<String>,
        metadata: serde_json::Value,
    ) -> Result<ChatMessage, ClientError> {
        let resp = self
            .request(
                FrameType::SendMessage,
                serde_json::to_value(SendMessagePayload {
                    room_id: room_id.to_string(),
                    content: self.encrypt_content(room_id, content),
                    reply_to: reply_to.map(String::from),
                    metadata,
                    mentions,
                })
                .unwrap(),
            )
            .await?;
        let mut msg: ChatMessage = serde_json::from_value(resp.payload).unwrap();
        self.decrypt_message(&mut msg);
        Ok(msg)
    }

    /// Broadcast a "thinking out loud" pulse to the room. Persisted to history
    /// (with `metadata.type = "thinking"`) so late-joining clients can see prior
    /// reasoning, but does NOT advance the room's turn token and is broadcast as
    /// a `thinking` event rather than `message_received`.
    pub async fn thinking(&self, room_id: &str, content: &str) -> Result<ChatMessage, ClientError> {
        let resp = self
            .request(
                FrameType::Thinking,
                serde_json::to_value(ThinkingPayload {
                    room_id: room_id.to_string(),
                    content: self.encrypt_content(room_id, content),
                })
                .unwrap(),
            )
            .await?;
        let mut msg: ChatMessage = serde_json::from_value(resp.payload).unwrap();
        self.decrypt_message(&mut msg);
        Ok(msg)
    }

    pub async fn get_history(
        &self,
        room_id: &str,
        limit: u32,
        before: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<ChatMessage>, ClientError> {
        self.get_history_filtered(room_id, limit, before, None, None)
            .await
    }

    /// Get history with optional `since` filter (returns messages after the given message_id).
    pub async fn get_history_since(
        &self,
        room_id: &str,
        limit: u32,
        before: Option<chrono::DateTime<chrono::Utc>>,
        since: Option<&str>,
    ) -> Result<Vec<ChatMessage>, ClientError> {
        self.get_history_filtered(room_id, limit, before, since, None)
            .await
    }

    /// Get history with optional filters. `since_seq` returns messages with seq strictly
    /// greater than the given value, ordered ascending by seq.
    pub async fn get_history_filtered(
        &self,
        room_id: &str,
        limit: u32,
        before: Option<chrono::DateTime<chrono::Utc>>,
        since: Option<&str>,
        since_seq: Option<i64>,
    ) -> Result<Vec<ChatMessage>, ClientError> {
        let resp = self
            .request(
                FrameType::GetHistory,
                serde_json::to_value(GetHistoryPayload {
                    room_id: room_id.to_string(),
                    limit,
                    before,
                    since: since.map(String::from),
                    since_seq,
                })
                .unwrap(),
            )
            .await?;
        let mut messages: Vec<ChatMessage> = resp
            .payload
            .get("messages")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        for msg in &mut messages {
            self.decrypt_message(msg);
        }
        Ok(messages)
    }

    /// Fetch one bounded, exactly contiguous history page after `after_seq`,
    /// never reading beyond the previously captured `through_seq`. A retained
    /// history hole is an error rather than permission to skip unread events.
    pub async fn get_contiguous_history_page(
        &self,
        room_id: &str,
        after_seq: i64,
        through_seq: i64,
        page_size: u32,
    ) -> Result<Vec<ChatMessage>, ClientError> {
        if after_seq > through_seq {
            return Err(ClientError::HistoryCursorAhead {
                room_id: room_id.to_string(),
                after_seq,
                through_seq,
            });
        }
        if after_seq == through_seq {
            return Ok(Vec::new());
        }

        let remaining = through_seq.saturating_sub(after_seq).max(1) as u64;
        let requested = u64::from(page_size.max(1)).min(remaining) as u32;
        let page = self
            .get_history_filtered(room_id, requested, None, None, Some(after_seq))
            .await?;
        let page: Vec<_> = page
            .into_iter()
            .take_while(|message| message.seq <= through_seq)
            .collect();

        let mut expected =
            after_seq
                .checked_add(1)
                .ok_or_else(|| ClientError::HistoryCursorAhead {
                    room_id: room_id.to_string(),
                    after_seq,
                    through_seq,
                })?;
        if page.is_empty() {
            return Err(ClientError::HistoryGap {
                room_id: room_id.to_string(),
                expected,
                found: None,
            });
        }
        for message in &page {
            if message.seq != expected {
                return Err(ClientError::HistoryGap {
                    room_id: room_id.to_string(),
                    expected,
                    found: Some(message.seq),
                });
            }
            expected = expected
                .checked_add(1)
                .ok_or_else(|| ClientError::HistoryGap {
                    room_id: room_id.to_string(),
                    expected,
                    found: None,
                })?;
        }
        if page.len() < requested as usize
            && page.last().is_some_and(|message| message.seq < through_seq)
        {
            return Err(ClientError::HistoryGap {
                room_id: room_id.to_string(),
                expected,
                found: None,
            });
        }
        Ok(page)
    }

    /// Return the latest seq for a room, or 0 if the room has no messages.
    pub async fn room_tip(&self, room_id: &str) -> Result<i64, ClientError> {
        let resp = self
            .request(FrameType::RoomTip, serde_json::json!({"room_id": room_id}))
            .await?;
        Ok(resp
            .payload
            .get("seq")
            .and_then(|v| v.as_i64())
            .unwrap_or(0))
    }

    pub async fn list_rooms(&self, parent_id: Option<&str>) -> Result<Vec<Room>, ClientError> {
        let resp = self
            .request(
                FrameType::ListRooms,
                serde_json::json!({"parent_id": parent_id}),
            )
            .await?;
        let rooms: Vec<Room> = resp
            .payload
            .get("rooms")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        Ok(rooms)
    }

    pub async fn list_agents(&self, room_id: Option<&str>) -> Result<Vec<AgentInfo>, ClientError> {
        let resp = self
            .request(
                FrameType::ListAgents,
                serde_json::json!({"room_id": room_id}),
            )
            .await?;
        let agents: Vec<AgentInfo> = resp
            .payload
            .get("agents")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        Ok(agents)
    }

    pub async fn room_info(&self, room_id: &str) -> Result<serde_json::Value, ClientError> {
        let resp = self
            .request(FrameType::RoomInfo, serde_json::json!({"room_id": room_id}))
            .await?;
        Ok(resp.payload)
    }

    /// Convenience: return the agent_id currently holding the turn token in `room_id`,
    /// or None if the room is empty.
    pub async fn current_turn_holder(&self, room_id: &str) -> Result<Option<String>, ClientError> {
        let info = self.room_info(room_id).await?;
        Ok(info
            .get("current_turn_holder")
            .and_then(|v| v.as_str())
            .map(String::from))
    }

    // --- Voting API ---

    /// Create a sealed-ballot vote in a room.
    pub async fn create_vote(
        &self,
        room_id: &str,
        title: &str,
        description: Option<&str>,
        options: Vec<String>,
        duration_secs: Option<u64>,
    ) -> Result<VoteInfo, ClientError> {
        let resp = self
            .request(
                FrameType::CreateVote,
                serde_json::to_value(CreateVotePayload {
                    room_id: room_id.to_string(),
                    title: title.to_string(),
                    description: description.map(String::from),
                    options,
                    duration_secs,
                })
                .unwrap(),
            )
            .await?;
        Ok(serde_json::from_value(resp.payload).unwrap())
    }

    /// Cast a ballot in an active vote (sealed until vote closes).
    pub async fn cast_vote(
        &self,
        vote_id: &str,
        option_index: usize,
    ) -> Result<serde_json::Value, ClientError> {
        let resp = self
            .request(
                FrameType::CastVote,
                serde_json::to_value(CastVotePayload {
                    vote_id: vote_id.to_string(),
                    option_index,
                })
                .unwrap(),
            )
            .await?;
        Ok(resp.payload)
    }

    /// Get the current status of a vote.
    ///
    /// For open votes this reports counts only. For closed votes it also includes
    /// revealed tally data.
    pub async fn get_vote_status(&self, vote_id: &str) -> Result<VoteInfo, ClientError> {
        let resp = self
            .request(
                FrameType::GetVoteStatus,
                serde_json::to_value(GetVoteStatusPayload {
                    vote_id: vote_id.to_string(),
                })
                .unwrap(),
            )
            .await?;
        Ok(serde_json::from_value(resp.payload).unwrap())
    }

    /// List recent votes for a room (open and/or closed).
    pub async fn list_votes(
        &self,
        room_id: &str,
        limit: u32,
    ) -> Result<Vec<VoteInfo>, ClientError> {
        let resp = self
            .request(
                FrameType::ListVotes,
                serde_json::to_value(ListVotesPayload {
                    room_id: room_id.to_string(),
                    limit,
                })
                .unwrap(),
            )
            .await?;

        let votes: Vec<VoteInfo> = resp
            .payload
            .get("votes")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        Ok(votes)
    }

    // --- Election API ---

    /// Start a leader election in a room.
    pub async fn elect_leader(&self, room_id: &str) -> Result<serde_json::Value, ClientError> {
        let resp = self
            .request(
                FrameType::ElectLeader,
                serde_json::to_value(ElectLeaderPayload {
                    room_id: room_id.to_string(),
                })
                .unwrap(),
            )
            .await?;
        Ok(resp.payload)
    }

    /// Decline an active election (opt out of candidacy).
    pub async fn decline_election(&self, room_id: &str) -> Result<serde_json::Value, ClientError> {
        let resp = self
            .request(
                FrameType::DeclineElection,
                serde_json::to_value(DeclineElectionPayload {
                    room_id: room_id.to_string(),
                })
                .unwrap(),
            )
            .await?;
        Ok(resp.payload)
    }

    /// Issue a decision as the room leader.
    pub async fn send_decision(
        &self,
        room_id: &str,
        content: &str,
        metadata: serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        let resp = self
            .request(
                FrameType::Decision,
                serde_json::to_value(DecisionPayload {
                    room_id: room_id.to_string(),
                    content: self.encrypt_content(room_id, content),
                    metadata,
                })
                .unwrap(),
            )
            .await?;
        Ok(resp.payload)
    }

    // --- Presence API ---

    /// Signal that this agent is typing (or stopped typing) in a room.
    pub async fn set_typing(&self, room_id: &str, typing: bool) -> Result<(), ClientError> {
        self.request(
            FrameType::SetTyping,
            serde_json::json!({"room_id": room_id, "typing": typing}),
        )
        .await?;
        Ok(())
    }

    /// Set this agent's presence status. Status must be "idle", "waiting", "working", or "thinking".
    pub async fn set_presence(
        &self,
        status: &str,
        detail: Option<&str>,
        progress: Option<u8>,
    ) -> Result<(), ClientError> {
        self.request(
            FrameType::SetPresence,
            serde_json::json!({
                "status": status,
                "status_detail": detail,
                "progress": progress,
            }),
        )
        .await?;
        Ok(())
    }

    /// Subscribe to server-pushed events (messages, joins, leaves, etc.)
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        match self.event_tx.upgrade() {
            Some(sender) => sender.subscribe(),
            None => {
                let (sender, receiver) = broadcast::channel(1);
                drop(sender);
                receiver
            }
        }
    }

    /// Wait for the next message in a specific room. Blocks until a message arrives or timeout.
    /// Returns the message, or None on timeout.
    ///
    /// If `since_seq` is set, also catches up on backlog: subscribes first, then queries
    /// history for any message with `seq > since_seq`. If the backlog is non-empty, returns
    /// the oldest such message immediately (callers can update their bookmark to its seq and
    /// call again to drain). Otherwise blocks on new events. This closes the race where a
    /// peer's reply arrives between two `wait` invocations and is silently missed.
    pub async fn wait_for_message(
        &self,
        room_id: &str,
        timeout_secs: u64,
        since_seq: Option<i64>,
    ) -> Result<Option<ChatMessage>, ClientError> {
        // Subscribe FIRST so we don't miss anything that arrives while we're fetching history.
        let mut events = self.subscribe();

        if let Some(seq) = since_seq {
            let tip = self.room_tip(room_id).await?;
            if seq > tip {
                return Err(ClientError::HistoryCursorAhead {
                    room_id: room_id.to_string(),
                    after_seq: seq,
                    through_seq: tip,
                });
            }
            if let Some(message) = self.oldest_wait_backlog_message(room_id, seq, tip).await? {
                return Ok(Some(message));
            }
        }

        self.wait_for_live_message(room_id, timeout_secs, since_seq, &mut events)
            .await
    }

    /// Return the oldest persisted peer chat after `seq`, paging past arbitrary
    /// thinking/system/self noise. Keeping this as a separate operation lets a
    /// lagged broadcast receiver repeat the same authoritative backfill rather
    /// than accepting a later retained event and creating a cursor gap.
    async fn oldest_wait_backlog_message(
        &self,
        room_id: &str,
        seq: i64,
        through_seq: i64,
    ) -> Result<Option<ChatMessage>, ClientError> {
        const BACKLOG_PAGE_SIZE: u32 = 32;
        let mut history_cursor = seq;
        while history_cursor < through_seq {
            let backlog = self
                .get_contiguous_history_page(
                    room_id,
                    history_cursor,
                    through_seq,
                    BACKLOG_PAGE_SIZE,
                )
                .await?;
            let page_tip = backlog.last().map(|message| message.seq);

            if let Some(message) = backlog.into_iter().find(|message| {
                let meta_type = message
                    .metadata
                    .get("type")
                    .and_then(|value| value.as_str());
                meta_type != Some("thinking")
                    && meta_type != Some("system")
                    && !self.is_self_message(message)
            }) {
                return Ok(Some(message));
            }

            let Some(next_cursor) = page_tip else {
                return Err(ClientError::HistoryGap {
                    room_id: room_id.to_string(),
                    expected: history_cursor.saturating_add(1),
                    found: None,
                });
            };
            history_cursor = next_cursor;
        }
        Ok(None)
    }

    async fn wait_for_live_message(
        &self,
        room_id: &str,
        timeout_secs: u64,
        since_seq: Option<i64>,
        events: &mut broadcast::Receiver<Event>,
    ) -> Result<Option<ChatMessage>, ClientError> {
        let deadline = tokio::time::sleep(std::time::Duration::from_secs(timeout_secs));
        tokio::pin!(deadline);

        loop {
            tokio::select! {
                _ = &mut deadline => {
                    return Ok(None);
                }
                event = events.recv() => {
                    match event {
                        Ok(evt) => {
                            if evt.frame.frame_type == FrameType::MessageReceived {
                                if let Some(event_room) = evt.frame.payload.get("room_id").and_then(|v| v.as_str()) {
                                    if event_room == room_id {
                                        let mut msg: ChatMessage = serde_json::from_value(evt.frame.payload)
                                            .map_err(ClientError::Json)?;
                                        if let Some(seq) = since_seq {
                                            if msg.seq <= seq {
                                                continue;
                                            }
                                            if msg.seq != seq.saturating_add(1) {
                                                // A later live event is only safe
                                                // after authoritative history has
                                                // accounted for every intervening
                                                // sequence (including filtered
                                                // self/system/thinking rows).
                                                let tip = self.room_tip(room_id).await?;
                                                if let Some(message) = self
                                                    .oldest_wait_backlog_message(room_id, seq, tip)
                                                    .await?
                                                {
                                                    return Ok(Some(message));
                                                }
                                                continue;
                                            }
                                        }
                                        // Same filter as the backlog path: skip
                                        // system messages and anything from the
                                        // same --name (different connection's
                                        // post by this same logical agent).
                                        let meta_type =
                                            msg.metadata.get("type").and_then(|v| v.as_str());
                                        let is_system = meta_type == Some("system");
                                        let is_self = self.is_self_message(&msg);
                                        if is_system || is_self {
                                            continue;
                                        }
                                        self.decrypt_message(&mut msg);
                                        return Ok(Some(msg));
                                    }
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            return Err(ClientError::ConnectionClosed);
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            if let Some(seq) = since_seq {
                                let tip = self.room_tip(room_id).await?;
                                if let Some(message) = self
                                    .oldest_wait_backlog_message(room_id, seq, tip)
                                    .await?
                                {
                                    return Ok(Some(message));
                                }
                            }
                            return Err(ClientError::EventStreamLagged { skipped });
                        }
                    }
                }
            }
        }
    }

    // --- Webhook subscriptions ---

    /// Register a webhook subscription. Returns the created `Subscription`
    /// (without the secret, which only the caller knows).
    #[allow(clippy::too_many_arguments)]
    pub async fn create_subscription(
        &self,
        room_id: &str,
        webhook_url: &str,
        secret: &str,
        kinds: Vec<String>,
        only_from: Option<&str>,
        not_from: Option<&str>,
        exclude_thinking: bool,
        since_seq: Option<i64>,
    ) -> Result<Subscription, ClientError> {
        let resp = self
            .request(
                FrameType::Subscribe,
                serde_json::to_value(SubscribePayload {
                    room_id: room_id.to_string(),
                    webhook_url: webhook_url.to_string(),
                    secret: secret.to_string(),
                    kinds,
                    only_from: only_from.map(String::from),
                    not_from: not_from.map(String::from),
                    exclude_thinking,
                    since_seq,
                })
                .unwrap(),
            )
            .await?;
        serde_json::from_value(resp.payload).map_err(ClientError::Json)
    }

    pub async fn unsubscribe(&self, subscription_id: &str) -> Result<(), ClientError> {
        self.request(
            FrameType::Unsubscribe,
            serde_json::to_value(UnsubscribePayload {
                subscription_id: subscription_id.to_string(),
            })
            .unwrap(),
        )
        .await?;
        Ok(())
    }

    pub async fn list_subscriptions(
        &self,
        room_id: Option<&str>,
    ) -> Result<Vec<Subscription>, ClientError> {
        let resp = self
            .request(
                FrameType::ListSubscriptions,
                serde_json::to_value(ListSubscriptionsPayload {
                    room_id: room_id.map(String::from),
                })
                .unwrap(),
            )
            .await?;
        let subs = resp
            .payload
            .get("subscriptions")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]));
        serde_json::from_value(subs).map_err(ClientError::Json)
    }

    pub async fn enable_subscription(&self, subscription_id: &str) -> Result<(), ClientError> {
        self.request(
            FrameType::EnableSubscription,
            serde_json::to_value(EnableSubscriptionPayload {
                subscription_id: subscription_id.to_string(),
            })
            .unwrap(),
        )
        .await?;
        Ok(())
    }

    /// Loop `wait_for_message` until a real chat message arrives.
    ///
    /// Each iteration uses `inner_timeout_secs` as its block budget; on timeout the
    /// loop retries with the same `since_seq` (so a peer's message that lands between
    /// iterations is still caught via the backlog path on the next iteration). The
    /// returned `ChatMessage` is the first new chat the room sees. This is the call
    /// agents should make when they want "wait until something happens" semantics
    /// without managing the re-poll discipline themselves; the CLI exposes it as
    /// `wait --loop`.
    pub async fn wait_for_message_loop(
        &self,
        room_id: &str,
        inner_timeout_secs: u64,
        since_seq: Option<i64>,
    ) -> Result<ChatMessage, ClientError> {
        loop {
            if let Some(msg) = self
                .wait_for_message(room_id, inner_timeout_secs, since_seq)
                .await?
            {
                return Ok(msg);
            }
        }
    }
}
