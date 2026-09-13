use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{timeout, Duration},
};

const ROOM: &str = "00000000-0000-4000-8000-000000000001";
const TRIGGER: &str = "00000000-0000-4000-8000-000000000002";
// Public RFC 8032 test vector, never an operational credential.
fn hex32(s: &str) -> [u8; 32] {
    std::array::from_fn(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
}
fn seed() -> [u8; 32] {
    hex32("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
}
fn seat() -> RoomSeat {
    RoomSeat {
        room: ROOM.into(),
        chain_id: 1,
        transport_generation: 0,
        key_generation: 1,
        seat: "fixture".into(),
        role: "owner".into(),
        certificate: "fixture-cert".into(),
        public_key: hex32("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"),
    }
}
fn reply(client: &SeatedHttpClient) -> PreparedReply {
    let wake = VerifiedWake {
        data: WakeData {
            room: ROOM.into(),
            message_id: TRIGGER.into(),
            seq: 1,
            tip: 1,
            since_seq: 0,
            dispatch_id: "fixture".into(),
            transport_generation: 0,
        },
    };
    client
        .prepare_reply(&wake, b"fixture reply", &seed(), &[42; 32])
        .unwrap()
}
struct FencedSigner {
    revoked: Arc<AtomicBool>,
    checks: AtomicUsize,
    signatures: AtomicUsize,
}
impl SeatedRequestSigner for FencedSigner {
    fn sign_request(&self, projection: &[u8]) -> Result<Vec<u8>, RoomError> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        if self.revoked.load(Ordering::SeqCst) {
            return Err(RoomError::Refused(403));
        }
        self.signatures.fetch_add(1, Ordering::SeqCst);
        SeedSigner(&seed()).sign_request(projection)
    }
}
fn signer(revoked: Arc<AtomicBool>) -> FencedSigner {
    FencedSigner {
        revoked,
        checks: AtomicUsize::new(0),
        signatures: AtomicUsize::new(0),
    }
}
async fn accept_request(listener: &TcpListener) -> (TcpStream, String) {
    let (mut stream, _) = timeout(Duration::from_secs(2), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut bytes = Vec::new();
    let split;
    loop {
        let byte = timeout(Duration::from_secs(2), stream.read_u8())
            .await
            .unwrap()
            .unwrap();
        bytes.push(byte);
        assert!(bytes.len() < 16 * 1024);
        if bytes.ends_with(b"\r\n\r\n") {
            split = bytes.len();
            break;
        }
    }
    let headers = String::from_utf8(bytes).unwrap();
    let header = |name: &str| {
        headers.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
    };
    let length: usize = header("content-length")
        .map(|v| v.parse().unwrap())
        .unwrap_or(0);
    assert!(length < 16 * 1024 && split > 0);
    let mut body = vec![0; length];
    timeout(Duration::from_secs(2), stream.read_exact(&mut body))
        .await
        .unwrap()
        .unwrap();
    let first = headers.lines().next().unwrap();
    let mut parts = first.split_whitespace();
    request::verify_received(
        &B64.decode(header("x-cowchat-request").unwrap()).unwrap(),
        &seat().public_key,
        &B64.decode(header("x-cowchat-signature").unwrap()).unwrap(),
        chrono::Utc::now().timestamp_millis() as u64,
        parts.next().unwrap(),
        parts.next().unwrap(),
        &body,
    )
    .unwrap();
    (stream, first.into())
}
async fn respond(stream: &mut TcpStream, status: &str, body: &Value) {
    let body = serde_json::to_vec(body).unwrap();
    stream
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    stream.write_all(&body).await.unwrap();
    stream.shutdown().await.unwrap();
}

#[tokio::test]
async fn denied_lease_never_sends_history_or_append() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = SeatedHttpClient::new(
        &format!("http://{}/", listener.local_addr().unwrap()),
        seat(),
    )
    .unwrap();
    let signer = signer(Arc::new(AtomicBool::new(true)));
    assert!(matches!(
        client
            .read_ciphertext_message_with_signer(TRIGGER, &signer)
            .await,
        Err(RoomError::Refused(403))
    ));
    assert!(matches!(
        client
            .submit_reply_with_signer(&reply(&client), &signer)
            .await,
        Err(RoomError::Refused(403))
    ));
    assert!(matches!(
        client.find_reply_with_signer(TRIGGER, &signer).await,
        Err(RoomError::Refused(403))
    ));
    assert_eq!(signer.checks.load(Ordering::SeqCst), 3);
    assert_eq!(signer.signatures.load(Ordering::SeqCst), 0);
    assert!(timeout(Duration::from_millis(50), listener.accept())
        .await
        .is_err());
}

#[tokio::test]
async fn recovery_distinguishes_absence_from_invalid_or_refused_history() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = SeatedHttpClient::new(
        &format!("http://{}/", listener.local_addr().unwrap()),
        seat(),
    )
    .unwrap();
    let signer = signer(Arc::new(AtomicBool::new(false)));
    let candidate = reply(&client);
    let winner: Value = serde_json::from_slice(candidate.bytes()).unwrap();
    let id = candidate.message_id().to_owned();
    let mut forged = winner.clone();
    forged["sig"] = B64.encode([0; 64]).into();
    // A correctly signed record for a different reply cannot win either.
    let other = client
        .prepare_reply(
            &VerifiedWake {
                data: WakeData {
                    room: ROOM.into(),
                    message_id: "00000000-0000-4000-8000-000000000003".into(),
                    seq: 2,
                    tip: 2,
                    since_seq: 0,
                    dispatch_id: "fixture".into(),
                    transport_generation: 0,
                },
            },
            b"other",
            &seed(),
            &[42; 32],
        )
        .unwrap();
    let other: Value = serde_json::from_slice(other.bytes()).unwrap();
    let row = serde_json::json!({"position":2,"record":winner});
    let pages = vec![
        serde_json::json!({"records":[]}),
        serde_json::json!({}),
        serde_json::json!({"records":[{"position":2,"record":forged}]}),
        serde_json::json!({"records":[{"position":2,"record":other}]}),
        serde_json::json!({"records":[{"position":0,"record":winner}]}),
        serde_json::json!({"records":[row.clone(), row.clone()]}),
        serde_json::json!({"records":[row]}),
    ];
    let server = tokio::spawn(async move {
        for page in pages {
            let (mut stream, line) = accept_request(&listener).await;
            assert_eq!(line, format!("GET /rooms/{ROOM}/messages?transport_generation=0&after=0&limit=1&message_id={id} HTTP/1.1"));
            respond(&mut stream, "200 OK", &page).await;
        }
        let (mut stream, _) = accept_request(&listener).await;
        respond(
            &mut stream,
            "403 Forbidden",
            &serde_json::json!({"records":[]}),
        )
        .await;
    });
    assert!(client
        .find_reply_with_signer(TRIGGER, &signer)
        .await
        .unwrap()
        .is_none());
    for _ in 0..5 {
        assert!(matches!(
            client.find_reply_with_signer(TRIGGER, &signer).await,
            Err(RoomError::Invalid)
        ));
    }
    let found = client
        .find_reply_with_signer(TRIGGER, &signer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.message_id(), candidate.message_id());
    assert_eq!(found.position(), 2);
    assert!(matches!(
        client.find_reply_with_signer(TRIGGER, &signer).await,
        Err(RoomError::Refused(403))
    ));
    assert!(matches!(
        client.find_reply_with_signer("bad&id", &signer).await,
        Err(RoomError::Invalid)
    ));
    assert_eq!(signer.signatures.load(Ordering::SeqCst), 8);
    server.await.unwrap();
}

#[tokio::test]
async fn revocation_during_append_prevents_conflict_read_signature_and_io() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = SeatedHttpClient::new(
        &format!("http://{}/", listener.local_addr().unwrap()),
        seat(),
    )
    .unwrap();
    let revoked = Arc::new(AtomicBool::new(false));
    let signer = signer(revoked.clone());
    let server = tokio::spawn(async move {
        let (mut stream, line) = accept_request(&listener).await;
        assert_eq!(line, format!("POST /rooms/{ROOM}/messages HTTP/1.1"));
        revoked.store(true, Ordering::SeqCst);
        respond(&mut stream, "409 Conflict", &serde_json::json!({})).await;
        assert!(timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err());
    });
    assert!(matches!(
        client
            .submit_reply_with_signer(&reply(&client), &signer)
            .await,
        Err(RoomError::Refused(403))
    ));
    assert_eq!(signer.checks.load(Ordering::SeqCst), 2);
    assert_eq!(signer.signatures.load(Ordering::SeqCst), 1);
    server.await.unwrap();
}

#[tokio::test]
async fn current_lease_reconciles_verified_winner_with_two_distinct_checks() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = SeatedHttpClient::new(
        &format!("http://{}/", listener.local_addr().unwrap()),
        seat(),
    )
    .unwrap();
    let reply = reply(&client);
    let winner: Value = serde_json::from_slice(reply.bytes()).unwrap();
    let id = reply.message_id().to_owned();
    let signer = signer(Arc::new(AtomicBool::new(false)));
    let server = tokio::spawn(async move {
        let (mut stream, _) = accept_request(&listener).await;
        respond(&mut stream, "409 Conflict", &serde_json::json!({})).await;
        let (mut stream, line) = accept_request(&listener).await;
        assert_eq!(line, format!("GET /rooms/{ROOM}/messages?transport_generation=0&after=0&limit=1&message_id={id} HTTP/1.1"));
        respond(
            &mut stream,
            "200 OK",
            &serde_json::json!({"records":[{"position":2,"record":winner}]}),
        )
        .await;
    });
    let result = client
        .submit_reply_with_signer(&reply, &signer)
        .await
        .unwrap();
    assert_eq!(result["status"], "existing");
    assert_eq!(result["message_id"], reply.message_id());
    assert_eq!(signer.checks.load(Ordering::SeqCst), 2);
    assert_eq!(signer.signatures.load(Ordering::SeqCst), 2);
    server.await.unwrap();
}
