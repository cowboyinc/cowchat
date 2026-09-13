//! Loopback M-ECHO fixture using the real Cowchat HTTP service, SQLite store and
//! webhook worker. Public test keys/context are seeded explicitly; this does NOT
//! replace or demonstrate production wallet/actor enrollment or key delivery.
use base64::{engine::general_purpose::STANDARD_NO_PAD as B64, Engine};
use ciborium::value::Value as Cbor;
use cowchat_client::seated::{RoomSeat, SeatedHttpClient};
use cowchat_crypto::{canonical, envelope, request};
use cowchat_server::{CowchatServer, ServerConfig};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, path::PathBuf};
use uuid::Uuid;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const ROOM: &str = "10000000-0000-4000-8000-000000000001";
const TRIGGER: &str = "20000000-0000-4000-8000-000000000001";
fn address(byte: u8) -> String {
    format!("0x{}", format!("{byte:02x}").repeat(20))
}
fn encode(value: &impl serde::Serialize) -> Result<Vec<u8>> {
    let mut bytes = vec![];
    ciborium::into_writer(value, &mut bytes)?;
    Ok(canonical::canonicalize(&bytes)?)
}
fn context(actor: bool) -> Result<Vec<u8>> {
    let (seat, role, cert, key) = if actor {
        (address(9), "actor", "actor-cert", 21)
    } else {
        (address(7), "owner", "owner-cert", 7)
    };
    let public = ed25519_dalek::SigningKey::from_bytes(&[key; 32])
        .verifying_key()
        .to_bytes();
    let value = json!({"chain_id":42,"room":ROOM,"gen":2,"seat":seat,"role":role,"cert":cert,"public_key":null,
        "rights":["read","write"],"expires_at":null,"door_kind":null,"bound_sender":null,"forwarded_seat":null});
    let Cbor::Map(mut fields) = ciborium::from_reader(encode(&value)?.as_slice())? else {
        unreachable!()
    };
    for (key, value) in &mut fields {
        if *key == Cbor::Text("public_key".into()) {
            *value = Cbor::Bytes(public.to_vec());
        }
    }
    encode(&Cbor::Map(fields))
}
fn loopback(origin: &str) -> Result<()> {
    let url: reqwest::Url = origin.parse()?;
    if url.scheme() != "http"
        || !url
            .host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback())
    {
        return Err("fixture endpoints must be literal HTTP loopback".into());
    }
    Ok(())
}
async fn signed(
    origin: &str,
    method: reqwest::Method,
    target: &str,
    body: Vec<u8>,
    actor: bool,
) -> Result<reqwest::Response> {
    loopback(origin)?;
    let projection = encode(&Cbor::Array(vec![
        Cbor::Text(method.as_str().into()),
        Cbor::Text(target.into()),
        Cbor::Bytes(Sha256::digest(&body).to_vec()),
        (chrono::Utc::now().timestamp_millis() as u64).into(),
        Cbor::Bytes(Uuid::new_v4().as_bytes().to_vec()),
    ]))?;
    let key = if actor { 21 } else { 7 };
    let sig = request::sign(&projection, &[key; 32])?;
    Ok(reqwest::Client::new()
        .request(method, format!("{origin}{target}"))
        .header(
            "x-cowchat-certificate",
            if actor { "actor-cert" } else { "owner-cert" },
        )
        .header("x-cowchat-request", B64.encode(projection))
        .header("x-cowchat-signature", B64.encode(sig))
        .body(body)
        .send()
        .await?)
}
fn sealed(plaintext: &[u8]) -> Result<Vec<u8>> {
    let header = json!({"v":3,"message_id":TRIGGER,"chain_id":42,"room":ROOM,"seat":address(7),"role":"owner","via":null,"via_sender":null,
        "class":"message","reply_to":null,"mentions":[address(9)],"wake_hint":"normal","gen":2,"cert":"owner-cert","nonce":B64.encode([0;12])});
    let bytes = envelope::seal(&encode(&header)?, &[42; 32], plaintext, &[7; 32])?;
    let Cbor::Array(parts) = ciborium::from_reader(bytes.as_slice())? else {
        unreachable!()
    };
    let [Cbor::Bytes(header), Cbor::Text(body), Cbor::Bytes(sig)] = parts.as_slice() else {
        unreachable!()
    };
    let mut record: Value = ciborium::from_reader(header.as_slice())?;
    record["body"] = body.clone().into();
    record["sig"] = B64.encode(sig).into();
    Ok(serde_json::to_vec(&record)?)
}
async fn serve(bind: &str, dir: PathBuf) -> Result<()> {
    let addr: SocketAddr = bind.parse()?;
    if !addr.ip().is_loopback() {
        return Err("fixture must bind loopback".into());
    }
    std::fs::create_dir_all(&dir)?;
    let db = dir.join("room.db");
    let server = CowchatServer::new(ServerConfig {
        socket_path: dir.join("fixture.sock"),
        tcp_addr: None,
        http_addr: Some(bind.into()),
        db_path: db.clone(),
        auth_key_path: dir.join("fixture-auth.key"),
        no_auth: false,
        allow_keyless_local: false,
        allow_private_webhooks: true,
        http_signup_enabled: false,
        http_admin_secret: None,
        http_allowed_origins: vec![],
        trusted_proxy_ips: vec![],
        blob_idle_expiry_seconds: 259_200,
    })?;
    if server.store().get_room(ROOM)?.is_none() {
        server
            .store()
            .create_room(ROOM, "M-ECHO fixture", None, None, Some("fixture"))?;
        let mut conn = rusqlite::Connection::open(&db)?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO seated_rooms(room_id,auth_generation,key_generation) VALUES (?1,1,2)",
            [ROOM],
        )?;
        for actor in [false, true] {
            let (seat, cert, key) = if actor {
                (address(9), "actor-cert", 21)
            } else {
                (address(7), "owner-cert", 7)
            };
            let public = ed25519_dalek::SigningKey::from_bytes(&[key; 32])
                .verifying_key()
                .to_bytes();
            tx.execute("INSERT INTO seated_credentials(room_id,cert_id,seat,display_name,auth_generation,public_key,trusted_context) VALUES (?1,?2,?3,?4,1,?5,?6)",
                rusqlite::params![ROOM,cert,seat,if actor {"Fixture actor"}else{"Fixture owner"},public.to_vec(),context(actor)?])?;
        }
        tx.commit()?;
    }
    std::fs::write(dir.join("owner-context.cbor"), context(false)?)?;
    println!(
        "{}",
        json!({"room_origin":format!("http://{bind}"),"room":ROOM,"fixture":true})
    );
    server.run().await
}
async fn send(origin: &str, gateway: &str, dir: PathBuf) -> Result<()> {
    loopback(gateway)?;
    let subscription = json!({"subscription_id":"30000000-0000-4000-8000-000000000001","transport_generation":0,"webhook_url":gateway,"secret":"K".repeat(32),"after":0});
    let response = signed(
        origin,
        reqwest::Method::POST,
        &format!("/rooms/{ROOM}/subscriptions"),
        serde_json::to_vec(&subscription)?,
        true,
    )
    .await?;
    if !response.status().is_success() {
        return Err(format!("subscription refused: {}", response.status()).into());
    }
    let record = sealed(b"fixture private trigger")?;
    std::fs::write(dir.join("trigger.json"), &record)?;
    let target = format!("/rooms/{ROOM}/messages");
    let first = signed(
        origin,
        reqwest::Method::POST,
        &target,
        record.clone(),
        false,
    )
    .await?;
    if !first.status().is_success() {
        return Err(format!("append refused: {}", first.status()).into());
    }
    let first: Value = first.json().await?;
    let retry = signed(origin, reqwest::Method::POST, &target, record, false).await?;
    if !retry.status().is_success() {
        return Err("exact append retry refused".into());
    }
    let retry: Value = retry.json().await?;
    if first != retry {
        return Err("exact append retry changed receipt".into());
    }
    let conflict = signed(
        origin,
        reqwest::Method::POST,
        &target,
        sealed(b"independent candidate")?,
        false,
    )
    .await?;
    if conflict.status().as_u16() != 409 {
        return Err("different ciphertext was not refused".into());
    }
    let conn = rusqlite::Connection::open(dir.join("room.db"))?;
    let dispatch:String=conn.query_row("SELECT delivery_id FROM subscription_deliveries WHERE subscription_id=?1 AND message_id=?2",
        rusqlite::params!["30000000-0000-4000-8000-000000000001",TRIGGER],|r|r.get(0))?;
    println!(
        "{}",
        json!({"message_id":TRIGGER,"dispatch_id":dispatch,"same_bytes_retry":true,"changed_bytes_conflict":true,"fixture":true})
    );
    Ok(())
}
async fn verify(origin: &str, dir: PathBuf) -> Result<()> {
    loopback(origin)?;
    let public = ed25519_dalek::SigningKey::from_bytes(&[21; 32])
        .verifying_key()
        .to_bytes();
    let client = SeatedHttpClient::new(
        origin,
        RoomSeat {
            room: ROOM.into(),
            chain_id: 42,
            transport_generation: 0,
            key_generation: 2,
            seat: address(9),
            role: "actor".into(),
            certificate: "actor-cert".into(),
            public_key: public,
        },
    )?;
    let page = client.read_ciphertext_page(0, &[21; 32]).await?;
    let records = page["records"].as_array().ok_or("missing records")?;
    if records.len() != 2 {
        return Err(format!("expected exactly request+reply, found {}", records.len()).into());
    }
    let record = &records[1]["record"];
    if record["seat"] != address(9) || record["role"] != "actor" || record["reply_to"] != TRIGGER {
        return Err("reply identity/scope mismatch".into());
    }
    let mut header = record.clone();
    let map = header.as_object_mut().ok_or("bad record")?;
    map.remove("body");
    map.remove("sig");
    let plain = envelope::open(
        &encode(&header)?,
        record["body"].as_str().ok_or("body")?,
        &public,
        &B64.decode(record["sig"].as_str().ok_or("sig")?)?,
        &[42; 32],
    )?;
    if plain != b"fixture actor reply" {
        return Err("unexpected authenticated reply".into());
    }
    let conn = rusqlite::Connection::open(dir.join("room.db"))?;
    let pending: i64 = conn.query_row(
        "SELECT count(*) FROM subscription_deliveries WHERE subscription_id=?1",
        ["30000000-0000-4000-8000-000000000001"],
        |r| r.get(0),
    )?;
    if pending != 0 {
        return Err("wake acknowledgement not yet committed".into());
    }
    println!(
        "{}",
        json!({"request":TRIGGER,"reply":record["message_id"],"records":2,"signature_verified":true,"decrypted_locally":true,"pending_wakes":pending,"fixture":true})
    );
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str){
        Some("serve") if args.len()==4=>serve(&args[2],args[3].clone().into()).await,
        Some("send") if args.len()==5=>send(&args[2],&args[3],args[4].clone().into()).await,
        Some("verify") if args.len()==4=>verify(&args[2],args[3].clone().into()).await,
        _=>Err("usage: cowchat_room_fixture serve <loopback-bind> <temporary-state-dir> | send <room-origin> <gateway-wake-url> <state-dir> | verify <room-origin> <state-dir>".into()),
    }
}
