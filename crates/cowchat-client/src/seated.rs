//! Short-lived signed room operations. Private signing/decryption material is
//! borrowed for each operation and is never retained by the HTTP client.
//! Provisioning and invocation/billing authorization belong to the runtime.
use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD as B64},
    Engine,
};
use ciborium::value::Value as Cbor;
use cowchat_crypto::{canonical, envelope, request};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum RoomError {
    #[error("invalid room request or signed record")]
    Invalid,
    #[error("wake authentication failed")]
    WakeAuthentication,
    #[error("room HTTP request failed or outcome is unknown")]
    Transport,
    #[error("room request refused ({0})")]
    Refused(u16),
    #[error("conflicting reply has no authenticated winner")]
    UnresolvedConflict,
}
#[derive(Clone)]
pub struct RoomSeat {
    pub room: String,
    pub chain_id: u64,
    pub transport_generation: u64,
    pub key_generation: u64,
    pub seat: String,
    pub role: String,
    pub certificate: String,
    pub public_key: [u8; 32],
}
pub struct SeatedHttpClient {
    origin: reqwest::Url,
    http: reqwest::Client,
    seat: RoomSeat,
}
pub struct PreparedReply {
    trigger: String,
    message_id: String,
    bytes: Vec<u8>,
}
impl PreparedReply {
    /// Persist these ciphertext bytes before retrying the same candidate.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn message_id(&self) -> &str {
        &self.message_id
    }
}
pub struct VerifiedWake {
    data: WakeData,
}
impl VerifiedWake {
    pub fn message_id(&self) -> &str {
        &self.data.message_id
    }
    pub fn dispatch_id(&self) -> &str {
        &self.data.dispatch_id
    }
    pub fn since_seq(&self) -> i64 {
        self.data.since_seq
    }
    pub fn tip(&self) -> i64 {
        self.data.tip
    }
    pub fn seq(&self) -> i64 {
        self.data.seq
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WakeEvent {
    specversion: String,
    id: String,
    source: String,
    #[serde(rename = "type")]
    kind: String,
    datacontenttype: String,
    data: WakeData,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WakeData {
    room: String,
    message_id: String,
    seq: i64,
    tip: i64,
    since_seq: i64,
    dispatch_id: String,
    transport_generation: u64,
}

/// Verify Standard Webhooks bytes before parsing or using a wake. The secret
/// must come from this seat's authenticated runtime binding, not the request.
/// Successful notification authentication does not authorize paid execution.
pub fn verify_wake(
    headers: &[(String, String)],
    body: &[u8],
    secret: &[u8],
    room: &str,
    generation: u64,
    now_seconds: i64,
) -> Result<VerifiedWake, RoomError> {
    let invalid = || RoomError::WakeAuthentication;
    if body.len() > 16 * 1024 || secret.len() < 32 || now_seconds < 0 {
        return Err(invalid());
    }
    let header = |name: &str| {
        let mut values = headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case(name));
        let value = values.next().ok_or_else(invalid)?.1.as_str();
        if values.next().is_some() {
            return Err(invalid());
        }
        Ok(value)
    };
    let id = header("webhook-id")?;
    let timestamp = header("webhook-timestamp")?;
    let signatures = header("webhook-signature")?;
    if id.is_empty() || id.len() > 128 || timestamp.len() > 20 || signatures.len() > 1024 {
        return Err(invalid());
    }
    let time: i64 = timestamp.parse().map_err(|_| invalid())?;
    if time < 0 || now_seconds.abs_diff(time) > 300 {
        return Err(invalid());
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| invalid())?;
    mac.update(id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    let valid = signatures
        .split_ascii_whitespace()
        .filter_map(|v| v.strip_prefix("v1,"))
        .filter_map(|v| STANDARD.decode(v).ok())
        .any(|signature| mac.clone().verify_slice(&signature).is_ok());
    if !valid {
        return Err(invalid());
    }
    let event: WakeEvent = serde_json::from_slice(body).map_err(|_| invalid())?;
    let d = &event.data;
    if event.specversion != "1.0"
        || event.kind != "cowchat.room.wake"
        || event.datacontenttype != "application/json"
        || event.source != format!("/rooms/{room}")
        || event.id != id
        || d.dispatch_id != id
        || d.room != room
        || d.transport_generation != generation
        || d.since_seq < 0
        || d.seq <= d.since_seq
        || d.tip < d.seq
        || Uuid::parse_str(&d.message_id)
            .map_err(|_| invalid())?
            .to_string()
            != d.message_id
    {
        return Err(invalid());
    }
    Ok(VerifiedWake { data: event.data })
}
fn cbor(value: &impl serde::Serialize) -> Result<Vec<u8>, RoomError> {
    let mut raw = Vec::new();
    ciborium::into_writer(value, &mut raw).map_err(|_| RoomError::Invalid)?;
    canonical::canonicalize(&raw).map_err(|_| RoomError::Invalid)
}
fn record_parts(record: &Value) -> Result<(Vec<u8>, String, Vec<u8>), RoomError> {
    let mut header = record.clone();
    let object = header.as_object_mut().ok_or(RoomError::Invalid)?;
    let body = object
        .remove("body")
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or(RoomError::Invalid)?;
    let signature = object
        .remove("sig")
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or(RoomError::Invalid)?;
    Ok((
        cbor(&header)?,
        body,
        B64.decode(signature).map_err(|_| RoomError::Invalid)?,
    ))
}
impl SeatedHttpClient {
    pub fn new(origin: &str, seat: RoomSeat) -> Result<Self, RoomError> {
        let origin: reqwest::Url = origin.parse().map_err(|_| RoomError::Invalid)?;
        let local = origin
            .host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        if (origin.scheme() != "https" && !(origin.scheme() == "http" && local))
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || Uuid::parse_str(&seat.room)
                .map_err(|_| RoomError::Invalid)?
                .to_string()
                != seat.room
        {
            return Err(RoomError::Invalid);
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|_| RoomError::Transport)?;
        Ok(Self { origin, http, seat })
    }
    fn reply_id(&self, trigger: &str) -> Result<String, RoomError> {
        let trigger = Uuid::parse_str(trigger).map_err(|_| RoomError::Invalid)?;
        let mut h = Sha256::new();
        h.update(b"cowchat/v3/actor-reply/0");
        h.update(self.seat.chain_id.to_be_bytes());
        h.update(
            Uuid::parse_str(&self.seat.room)
                .map_err(|_| RoomError::Invalid)?
                .as_bytes(),
        );
        h.update((self.seat.seat.len() as u64).to_be_bytes());
        h.update(self.seat.seat.as_bytes());
        h.update(trigger.as_bytes());
        let hash = h.finalize();
        let mut id = [0; 16];
        id.copy_from_slice(&hash[..16]);
        id[6] = (id[6] & 15) | 0x80;
        id[8] = (id[8] & 63) | 0x80;
        Ok(Uuid::from_bytes(id).to_string())
    }
    pub fn prepare_reply(
        &self,
        wake: &VerifiedWake,
        plaintext: &[u8],
        signing_seed: &[u8],
        room_secret: &[u8],
    ) -> Result<PreparedReply, RoomError> {
        if wake.data.room != self.seat.room
            || wake.data.transport_generation != self.seat.transport_generation
        {
            return Err(RoomError::Invalid);
        }
        let id = self.reply_id(wake.message_id())?;
        let header = serde_json::json!({"v":3,"message_id":id,"chain_id":self.seat.chain_id,"room":self.seat.room,"seat":self.seat.seat,"role":self.seat.role,
            "via":null,"via_sender":null,"class":"message","reply_to":wake.message_id(),"mentions":[],"wake_hint":"none","gen":self.seat.key_generation,"cert":self.seat.certificate,"nonce":B64.encode([0;12])});
        let sealed = envelope::seal(&cbor(&header)?, room_secret, plaintext, signing_seed)
            .map_err(|_| RoomError::Invalid)?;
        let Cbor::Array(parts) =
            ciborium::from_reader(sealed.as_slice()).map_err(|_| RoomError::Invalid)?
        else {
            return Err(RoomError::Invalid);
        };
        let [Cbor::Bytes(header), Cbor::Text(body), Cbor::Bytes(sig)] = parts.as_slice() else {
            return Err(RoomError::Invalid);
        };
        let mut record: Value =
            ciborium::from_reader(header.as_slice()).map_err(|_| RoomError::Invalid)?;
        record["body"] = body.clone().into();
        record["sig"] = B64.encode(sig).into();
        self.restore_reply(
            wake.message_id(),
            &serde_json::to_vec(&record).map_err(|_| RoomError::Invalid)?,
        )
    }
    pub fn restore_reply(&self, trigger: &str, bytes: &[u8]) -> Result<PreparedReply, RoomError> {
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(RoomError::Invalid);
        }
        let record: Value = serde_json::from_slice(bytes).map_err(|_| RoomError::Invalid)?;
        let id = self.reply_id(trigger)?;
        self.validate_winner(&record, trigger, &id)?;
        Ok(PreparedReply {
            trigger: trigger.into(),
            message_id: id,
            bytes: bytes.to_vec(),
        })
    }
    fn validate_winner(&self, record: &Value, trigger: &str, id: &str) -> Result<(), RoomError> {
        let (header, body, sig) = record_parts(record)?;
        envelope::verify_record(&header, &body, &self.seat.public_key, &sig)
            .map_err(|_| RoomError::Invalid)?;
        if record["room"] != self.seat.room
            || record["chain_id"] != self.seat.chain_id
            || record["seat"] != self.seat.seat
            || record["role"] != self.seat.role
            || record["cert"] != self.seat.certificate
            || record["gen"] != self.seat.key_generation
            || record["message_id"] != id
            || record["reply_to"] != trigger
            || record["class"] != "message"
        {
            return Err(RoomError::Invalid);
        }
        Ok(())
    }
    async fn send(
        &self,
        method: reqwest::Method,
        target: &str,
        body: Vec<u8>,
        seed: &[u8],
    ) -> Result<reqwest::Response, RoomError> {
        let now =
            u64::try_from(chrono::Utc::now().timestamp_millis()).map_err(|_| RoomError::Invalid)?;
        let projection = cbor(&Cbor::Array(vec![
            Cbor::Text(method.as_str().into()),
            Cbor::Text(target.into()),
            Cbor::Bytes(Sha256::digest(&body).to_vec()),
            now.into(),
            Cbor::Bytes(Uuid::new_v4().as_bytes().to_vec()),
        ]))?;
        let signature = request::sign(&projection, seed).map_err(|_| RoomError::Invalid)?;
        self.http
            .request(
                method,
                self.origin.join(target).map_err(|_| RoomError::Invalid)?,
            )
            .header("x-cowchat-certificate", &self.seat.certificate)
            .header("x-cowchat-request", B64.encode(projection))
            .header("x-cowchat-signature", B64.encode(signature))
            .body(body)
            .send()
            .await
            .map_err(|_| RoomError::Transport)
    }
    /// HTTP authentication does not authenticate individual senders. Verify each
    /// record with independently provisioned sender credentials before opening.
    pub async fn read_ciphertext_page(&self, after: i64, seed: &[u8]) -> Result<Value, RoomError> {
        let target = format!(
            "/rooms/{}/messages?transport_generation={}&after={after}&limit=100",
            self.seat.room, self.seat.transport_generation
        );
        let response = self
            .send(reqwest::Method::GET, &target, vec![], seed)
            .await?;
        if !response.status().is_success() {
            return Err(RoomError::Refused(response.status().as_u16()));
        }
        bounded_json(response).await
    }

    /// Fetch one ciphertext record by ID within this seat's current read bounds.
    /// The caller must still authenticate the sender and signed record before use.
    pub async fn read_ciphertext_message(
        &self,
        message_id: &str,
        seed: &[u8],
    ) -> Result<Value, RoomError> {
        if Uuid::parse_str(message_id)
            .map_err(|_| RoomError::Invalid)?
            .to_string()
            != message_id
        {
            return Err(RoomError::Invalid);
        }
        let target = format!(
            "/rooms/{}/messages?transport_generation={}&after=0&limit=1&message_id={message_id}",
            self.seat.room, self.seat.transport_generation
        );
        let response = self
            .send(reqwest::Method::GET, &target, vec![], seed)
            .await?;
        if !response.status().is_success() {
            return Err(RoomError::Refused(response.status().as_u16()));
        }
        bounded_json(response).await
    }
    /// Retries send the identical sealed bytes under a fresh HTTP nonce. On a
    /// conflict, only a signature-verified record for this exact reply is success.
    pub async fn submit_reply(
        &self,
        reply: &PreparedReply,
        seed: &[u8],
    ) -> Result<Value, RoomError> {
        self.restore_reply(&reply.trigger, &reply.bytes)?;
        let target = format!("/rooms/{}/messages", self.seat.room);
        let response = self
            .send(reqwest::Method::POST, &target, reply.bytes.clone(), seed)
            .await?;
        let status = response.status();
        if status.is_success() {
            let receipt = bounded_json(response).await?;
            if receipt["message_id"] != reply.message_id
                || receipt["seq"].as_i64().is_none_or(|seq| seq <= 0)
            {
                return Err(RoomError::Invalid);
            }
            return Ok(receipt);
        }
        if status.as_u16() != 409 {
            return Err(RoomError::Refused(status.as_u16()));
        }
        let target = format!(
            "{target}?transport_generation={}&after=0&limit=1&message_id={}",
            self.seat.transport_generation, reply.message_id
        );
        let response = self
            .send(reqwest::Method::GET, &target, vec![], seed)
            .await?;
        if !response.status().is_success() {
            return Err(RoomError::UnresolvedConflict);
        }
        let page = bounded_json(response).await?;
        let rows = page["records"]
            .as_array()
            .ok_or(RoomError::UnresolvedConflict)?;
        if rows.len() != 1 || rows[0]["position"].as_i64().is_none_or(|seq| seq <= 0) {
            return Err(RoomError::UnresolvedConflict);
        }
        self.validate_winner(&rows[0]["record"], &reply.trigger, &reply.message_id)
            .map_err(|_| RoomError::UnresolvedConflict)?;
        Ok(
            serde_json::json!({"message_id":reply.message_id,"seq":rows[0]["position"],"status":"existing"}),
        )
    }
}
async fn bounded_json(mut response: reqwest::Response) -> Result<Value, RoomError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| RoomError::Transport)? {
        if bytes.len().saturating_add(chunk.len()) > 5 * 1024 * 1024 {
            return Err(RoomError::Invalid);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| RoomError::Invalid)
}
