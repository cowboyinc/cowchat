//! Loopback M-ECHO fixture using the real Cowchat HTTP service, SQLite store and
//! webhook worker. It consumes the composed Harness fixture's verified manifest;
//! this does not replace or demonstrate production enrollment or key delivery.
use base64::{engine::general_purpose::STANDARD_NO_PAD as B64, Engine};
use ciborium::value::Value as Cbor;
use cowchat_client::seated::{RoomSeat, SeatedHttpClient};
use cowchat_crypto::{canonical, envelope, request};
use cowchat_server::{CowchatServer, ServerConfig};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, net::SocketAddr, path::PathBuf};
use uuid::Uuid;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const ACTOR_TRIGGER: &str = "20000000-0000-4000-8000-000000000001";
const BUILDER_TRIGGER: &str = "20000000-0000-4000-8000-000000000002";
const ACTOR_SUBSCRIPTION: &str = "30000000-0000-4000-8000-000000000001";
const BUILDER_SUBSCRIPTION: &str = "30000000-0000-4000-8000-000000000002";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureManifest {
    run: Uuid,
    epoch_ms: u64,
    room: String,
    room_secret: String,
    webhook_secret: String,
    seats: Vec<FixtureSeat>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureSeat {
    room: String,
    chain_id: u64,
    transport_generation: u64,
    key_generation: u64,
    seat: String,
    role: String,
    certificate: String,
    public_key: String,
    context: String,
    signing_seed: String,
    gateway_actor: String,
}

fn decode32(value: &str, label: &str) -> Result<[u8; 32]> {
    B64.decode(value)?
        .try_into()
        .map_err(|_| format!("{label} is not 32 bytes").into())
}

fn context_text<'a>(fields: &'a [(Cbor, Cbor)], key: &str) -> Result<&'a str> {
    fields
        .iter()
        .find_map(|(candidate, value)| (*candidate == Cbor::Text(key.into())).then_some(value))
        .and_then(|value| match value {
            Cbor::Text(value) => Some(value.as_str()),
            _ => None,
        })
        .ok_or_else(|| format!("missing context {key}").into())
}

fn context_u64(fields: &[(Cbor, Cbor)], key: &str) -> Result<u64> {
    let value = fields
        .iter()
        .find_map(|(candidate, value)| (*candidate == Cbor::Text(key.into())).then_some(value))
        .ok_or_else(|| format!("missing context {key}"))?;
    let Cbor::Integer(value) = value else {
        return Err(format!("context {key} is not an integer").into());
    };
    Ok(u64::try_from(*value)?)
}

fn context_bytes<'a>(fields: &'a [(Cbor, Cbor)], key: &str) -> Result<&'a [u8]> {
    fields
        .iter()
        .find_map(|(candidate, value)| (*candidate == Cbor::Text(key.into())).then_some(value))
        .and_then(|value| match value {
            Cbor::Bytes(value) => Some(value.as_slice()),
            _ => None,
        })
        .ok_or_else(|| format!("missing context {key}").into())
}

impl FixtureSeat {
    fn public_key(&self) -> Result<[u8; 32]> {
        decode32(&self.public_key, "seat public key")
    }

    fn signing_seed(&self) -> Result<[u8; 32]> {
        decode32(&self.signing_seed, "seat signing seed")
    }

    fn context(&self) -> Result<Vec<u8>> {
        Ok(B64.decode(&self.context)?)
    }

    fn validate(&self, room: &str) -> Result<()> {
        if self.room != room
            || !matches!(self.role.as_str(), "actor" | "builder")
            || self.certificate.is_empty()
            || self.chain_id == 0
            || self.transport_generation == 0
            || self.key_generation == 0
        {
            return Err("invalid fixture seat metadata".into());
        }
        let raw = self
            .seat
            .strip_prefix("0x")
            .ok_or("fixture seat lacks 0x prefix")?;
        let (address, suffix) = raw
            .strip_suffix("#builder")
            .map_or((raw, ""), |address| (address, "#builder"));
        let address: [u8; 20] = hex::decode(address)?
            .try_into()
            .map_err(|_| "fixture seat is not 20 bytes")?;
        if address == [0; 20]
            || self.seat != format!("0x{}{suffix}", hex::encode(address))
            || (self.role == "builder") != (suffix == "#builder")
            || self.gateway_actor != format!("0x{}", hex::encode(address))
        {
            return Err("noncanonical fixture seat".into());
        }
        let public = self.public_key()?;
        if ed25519_dalek::SigningKey::from_bytes(&self.signing_seed()?)
            .verifying_key()
            .to_bytes()
            != public
        {
            return Err("fixture seat seed/public key mismatch".into());
        }
        let context = self.context()?;
        if canonical::canonicalize(&context)? != context {
            return Err("fixture seat context is not canonical CBOR".into());
        }
        let Cbor::Map(fields) = ciborium::from_reader(context.as_slice())? else {
            return Err("fixture seat context is not a map".into());
        };
        if context_text(&fields, "room")? != self.room
            || context_text(&fields, "seat")? != self.seat
            || context_text(&fields, "role")? != self.role
            || context_text(&fields, "cert")? != self.certificate
            || context_u64(&fields, "chain_id")? != self.chain_id
            || context_u64(&fields, "gen")? != self.key_generation
            || context_bytes(&fields, "public_key")? != public
        {
            return Err("fixture seat context disagrees with manifest".into());
        }
        Ok(())
    }

    fn room_seat(&self) -> Result<RoomSeat> {
        Ok(RoomSeat {
            room: self.room.clone(),
            chain_id: self.chain_id,
            transport_generation: self.transport_generation,
            key_generation: self.key_generation,
            seat: self.seat.clone(),
            role: self.role.clone(),
            certificate: self.certificate.clone(),
            public_key: self.public_key()?,
        })
    }
}

impl FixtureManifest {
    fn load(path: &PathBuf) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        let room = Uuid::parse_str(&manifest.room)?;
        if manifest.run.is_nil()
            || manifest.epoch_ms == 0
            || room.is_nil()
            || room.to_string() != manifest.room
            || manifest.seats.len() != 2
        {
            return Err("invalid fixture manifest identity".into());
        }
        decode32(&manifest.room_secret, "room secret")?;
        let webhook = decode32(&manifest.webhook_secret, "webhook secret")?;
        if std::str::from_utf8(&webhook).is_err() {
            return Err("fixture webhook secret must be UTF-8".into());
        }
        for seat in &manifest.seats {
            seat.validate(&manifest.room)?;
        }
        if manifest.seat("actor")?.chain_id != manifest.seat("builder")?.chain_id
            || manifest.seat("actor")?.transport_generation
                != manifest.seat("builder")?.transport_generation
            || manifest.seat("actor")?.key_generation != manifest.seat("builder")?.key_generation
        {
            return Err("fixture roles disagree on room generations".into());
        }
        Ok(manifest)
    }

    fn seat(&self, role: &str) -> Result<&FixtureSeat> {
        let matches = self
            .seats
            .iter()
            .filter(|seat| seat.role == role)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(format!("fixture requires exactly one {role} seat").into());
        }
        Ok(matches[0])
    }

    fn room_secret(&self) -> Result<[u8; 32]> {
        decode32(&self.room_secret, "room secret")
    }

    fn webhook_secret(&self) -> Result<String> {
        Ok(std::str::from_utf8(&decode32(&self.webhook_secret, "webhook secret")?)?.into())
    }
}

fn encode(value: &impl serde::Serialize) -> Result<Vec<u8>> {
    let mut bytes = vec![];
    ciborium::into_writer(value, &mut bytes)?;
    Ok(canonical::canonicalize(&bytes)?)
}

fn loopback(origin: &str) -> Result<()> {
    let url: reqwest::Url = origin.parse()?;
    if url.scheme() != "http"
        || !url
            .host_str()
            .and_then(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
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
    seat: &FixtureSeat,
) -> Result<reqwest::Response> {
    loopback(origin)?;
    let projection = encode(&Cbor::Array(vec![
        Cbor::Text(method.as_str().into()),
        Cbor::Text(target.into()),
        Cbor::Bytes(Sha256::digest(&body).to_vec()),
        (chrono::Utc::now().timestamp_millis() as u64).into(),
        Cbor::Bytes(Uuid::new_v4().as_bytes().to_vec()),
    ]))?;
    let signature = request::sign(&projection, &seat.signing_seed()?)?;
    Ok(reqwest::Client::new()
        .request(method, format!("{origin}{target}"))
        .header("x-cowchat-certificate", &seat.certificate)
        .header("x-cowchat-request", B64.encode(projection))
        .header("x-cowchat-signature", B64.encode(signature))
        .body(body)
        .send()
        .await?)
}

fn sealed(
    manifest: &FixtureManifest,
    sender: &FixtureSeat,
    target: &FixtureSeat,
    message_id: &str,
    nonce: u8,
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let header = json!({"v":3,"message_id":message_id,"chain_id":sender.chain_id,"room":manifest.room,
        "seat":sender.seat,"role":sender.role,"via":null,"via_sender":null,"class":"message",
        "reply_to":null,"mentions":[target.seat],"wake_hint":"normal","gen":sender.key_generation,
        "cert":sender.certificate,"nonce":B64.encode([nonce;12])});
    let bytes = envelope::seal(
        &encode(&header)?,
        &manifest.room_secret()?,
        plaintext,
        &sender.signing_seed()?,
    )?;
    let Cbor::Array(parts) = ciborium::from_reader(bytes.as_slice())? else {
        return Err("sealed record is not an array".into());
    };
    let [Cbor::Bytes(header), Cbor::Text(body), Cbor::Bytes(signature)] = parts.as_slice() else {
        return Err("sealed record has the wrong shape".into());
    };
    let mut record: Value = ciborium::from_reader(header.as_slice())?;
    record["body"] = body.clone().into();
    record["sig"] = B64.encode(signature).into();
    Ok(serde_json::to_vec(&record)?)
}

fn seed_database(db: &PathBuf, manifest: &FixtureManifest) -> Result<()> {
    let mut connection = rusqlite::Connection::open(db)?;
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM seated_rooms WHERE room_id=?1",
        [&manifest.room],
        |row| row.get(0),
    )?;
    if count == 0 {
        let transaction = connection.transaction()?;
        let actor = manifest.seat("actor")?;
        transaction.execute(
            "INSERT INTO seated_rooms(room_id,auth_generation,key_generation,transport_generation) VALUES (?1,1,?2,?3)",
            rusqlite::params![manifest.room, actor.key_generation, actor.transport_generation],
        )?;
        for seat in &manifest.seats {
            transaction.execute(
                "INSERT INTO seated_credentials(room_id,cert_id,seat,display_name,auth_generation,public_key,trusted_context) VALUES (?1,?2,?3,?4,1,?5,?6)",
                rusqlite::params![manifest.room,seat.certificate,seat.seat,format!("Fixture {}",seat.role),seat.public_key()?.to_vec(),seat.context()?],
            )?;
        }
        transaction.commit()?;
    }
    let (key, transport, credentials): (u64, u64, i64) = connection.query_row(
        "SELECT key_generation,transport_generation,(SELECT count(*) FROM seated_credentials WHERE room_id=?1) FROM seated_rooms WHERE room_id=?1 AND auth_generation=1",
        [&manifest.room],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let first = manifest.seat("actor")?;
    if key != first.key_generation || transport != first.transport_generation || credentials != 2 {
        return Err("persisted Cowchat fixture disagrees with manifest".into());
    }
    for seat in &manifest.seats {
        let (stored_seat, public, context): (String, Vec<u8>, Vec<u8>) = connection.query_row(
            "SELECT seat,public_key,trusted_context FROM seated_credentials WHERE room_id=?1 AND cert_id=?2 AND auth_generation=1",
            rusqlite::params![manifest.room, seat.certificate],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if stored_seat != seat.seat || public != seat.public_key()? || context != seat.context()? {
            return Err("persisted Cowchat seat disagrees with manifest".into());
        }
    }
    Ok(())
}

async fn serve(bind: &str, directory: PathBuf, manifest_path: PathBuf) -> Result<()> {
    let address: SocketAddr = bind.parse()?;
    if !address.ip().is_loopback() {
        return Err("fixture must bind loopback".into());
    }
    let manifest = FixtureManifest::load(&manifest_path)?;
    std::fs::create_dir_all(&directory)?;
    let db = directory.join("room.db");
    let server = CowchatServer::new(ServerConfig {
        socket_path: directory.join("fixture.sock"),
        tcp_addr: None,
        http_addr: Some(bind.into()),
        db_path: db.clone(),
        auth_key_path: directory.join("fixture-auth.key"),
        no_auth: false,
        allow_keyless_local: false,
        allow_private_webhooks: true,
        http_signup_enabled: false,
        http_admin_secret: None,
        http_allowed_origins: vec![],
        trusted_proxy_ips: vec![],
        blob_idle_expiry_seconds: 259_200,
    })?;
    if server.store().get_room(&manifest.room)?.is_none() {
        server.store().create_room(
            &manifest.room,
            "M-ECHO fixture",
            None,
            None,
            Some("fixture"),
        )?;
    }
    seed_database(&db, &manifest)?;
    println!(
        "{}",
        json!({"room_origin":format!("http://{bind}"),"room":manifest.room,"seats":[manifest.seat("actor")?.seat,manifest.seat("builder")?.seat],"fixture":true})
    );
    server.run().await
}

async fn install_subscription(
    origin: &str,
    gateway: &str,
    manifest: &FixtureManifest,
    seat: &FixtureSeat,
    subscription: &str,
) -> Result<()> {
    let webhook = format!("{gateway}/{}/wake", seat.gateway_actor);
    let body = json!({"subscription_id":subscription,"transport_generation":seat.transport_generation,
        "webhook_url":webhook,"secret":manifest.webhook_secret()?,"after":0});
    let response = signed(
        origin,
        reqwest::Method::POST,
        &format!("/rooms/{}/subscriptions", manifest.room),
        serde_json::to_vec(&body)?,
        seat,
    )
    .await?;
    if !response.status().is_success() {
        return Err(format!("{} subscription refused: {}", seat.role, response.status()).into());
    }
    Ok(())
}

async fn append_trigger(
    origin: &str,
    manifest: &FixtureManifest,
    sender: &FixtureSeat,
    target: &FixtureSeat,
    message_id: &str,
    plaintext: &[u8],
) -> Result<Value> {
    let record = sealed(manifest, sender, target, message_id, 1, plaintext)?;
    let path = format!("/rooms/{}/messages", manifest.room);
    let first = signed(origin, reqwest::Method::POST, &path, record.clone(), sender).await?;
    if !first.status().is_success() {
        return Err(format!("append refused: {}", first.status()).into());
    }
    let first: Value = first.json().await?;
    let retry = signed(origin, reqwest::Method::POST, &path, record, sender).await?;
    if !retry.status().is_success() || retry.json::<Value>().await? != first {
        return Err("exact append retry changed receipt".into());
    }
    let changed = sealed(
        manifest,
        sender,
        target,
        message_id,
        2,
        b"changed candidate",
    )?;
    let conflict = signed(origin, reqwest::Method::POST, &path, changed, sender).await?;
    if conflict.status().as_u16() != 409 {
        return Err("different ciphertext was not refused".into());
    }
    Ok(first)
}

async fn send(
    origin: &str,
    gateway: &str,
    directory: PathBuf,
    manifest_path: PathBuf,
) -> Result<()> {
    loopback(gateway)?;
    let manifest = FixtureManifest::load(&manifest_path)?;
    let actor = manifest.seat("actor")?;
    let builder = manifest.seat("builder")?;
    install_subscription(origin, gateway, &manifest, actor, ACTOR_SUBSCRIPTION).await?;
    install_subscription(origin, gateway, &manifest, builder, BUILDER_SUBSCRIPTION).await?;
    let actor_receipt = append_trigger(
        origin,
        &manifest,
        builder,
        actor,
        ACTOR_TRIGGER,
        b"fixture private trigger for actor",
    )
    .await?;
    let builder_receipt = append_trigger(
        origin,
        &manifest,
        actor,
        builder,
        BUILDER_TRIGGER,
        b"fixture private trigger for builder",
    )
    .await?;
    std::fs::write(
        directory.join("trigger-receipts.json"),
        serde_json::to_vec(&json!([actor_receipt, builder_receipt]))?,
    )?;
    let connection = rusqlite::Connection::open(directory.join("room.db"))?;
    let dispatch = |subscription: &str, message: &str| -> Result<String> {
        Ok(connection.query_row(
            "SELECT delivery_id FROM subscription_deliveries WHERE subscription_id=?1 AND message_id=?2",
            rusqlite::params![subscription, message],
            |row| row.get(0),
        )?)
    };
    println!(
        "{}",
        json!({"messages":[
            {"role":"actor","message_id":ACTOR_TRIGGER,"dispatch_id":dispatch(ACTOR_SUBSCRIPTION,ACTOR_TRIGGER)?},
            {"role":"builder","message_id":BUILDER_TRIGGER,"dispatch_id":dispatch(BUILDER_SUBSCRIPTION,BUILDER_TRIGGER)?}],
            "same_bytes_retry":true,"changed_bytes_conflict":true,"fixture":true})
    );
    Ok(())
}

fn open_record(record: &Value, manifest: &FixtureManifest) -> Result<Vec<u8>> {
    let seat_name = record["seat"].as_str().ok_or("record seat")?;
    let sender = manifest
        .seats
        .iter()
        .find(|seat| seat.seat == seat_name)
        .ok_or("record sender is not a fixture seat")?;
    if record["room"] != manifest.room
        || record["chain_id"] != sender.chain_id
        || record["role"] != sender.role
        || record["cert"] != sender.certificate
        || record["gen"] != sender.key_generation
    {
        return Err("record identity/scope mismatch".into());
    }
    let mut header = record.clone();
    let map = header.as_object_mut().ok_or("bad record")?;
    map.remove("body");
    map.remove("sig");
    Ok(envelope::open(
        &encode(&header)?,
        record["body"].as_str().ok_or("record body")?,
        &sender.public_key()?,
        &B64.decode(record["sig"].as_str().ok_or("record signature")?)?,
        &manifest.room_secret()?,
    )?)
}

async fn verify(origin: &str, directory: PathBuf, manifest_path: PathBuf) -> Result<()> {
    loopback(origin)?;
    let manifest = FixtureManifest::load(&manifest_path)?;
    let actor = manifest.seat("actor")?;
    let client = SeatedHttpClient::new(origin, actor.room_seat()?)?;
    let page = client
        .read_ciphertext_page(0, &actor.signing_seed()?)
        .await?;
    let records = page["records"].as_array().ok_or("missing records")?;
    if records.len() != 4 {
        return Err(format!("expected two requests and replies, found {}", records.len()).into());
    }
    let mut opened = HashMap::new();
    for item in records {
        let record = &item["record"];
        opened.insert(
            record["message_id"]
                .as_str()
                .ok_or("record message ID")?
                .to_owned(),
            (record.clone(), open_record(record, &manifest)?),
        );
    }
    let actor_trigger = opened.get(ACTOR_TRIGGER).ok_or("missing actor trigger")?;
    let builder_trigger = opened
        .get(BUILDER_TRIGGER)
        .ok_or("missing builder trigger")?;
    if actor_trigger.1 != b"fixture private trigger for actor"
        || builder_trigger.1 != b"fixture private trigger for builder"
    {
        return Err("unexpected authenticated trigger".into());
    }
    let reply = |trigger: &str, role: &str, plaintext: &[u8]| -> Result<String> {
        let matches = opened
            .iter()
            .filter(|(_, (record, body))| {
                record["reply_to"] == trigger && record["role"] == role && body == plaintext
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(format!("expected one authenticated {role} reply").into());
        }
        Ok(matches[0].0.clone())
    };
    let actor_reply = reply(ACTOR_TRIGGER, "actor", b"fixture actor reply")?;
    let builder_reply = reply(BUILDER_TRIGGER, "builder", b"fixture builder reply")?;
    let connection = rusqlite::Connection::open(directory.join("room.db"))?;
    let pending: i64 = connection.query_row(
        "SELECT count(*) FROM subscription_deliveries WHERE subscription_id IN (?1,?2)",
        [ACTOR_SUBSCRIPTION, BUILDER_SUBSCRIPTION],
        |row| row.get(0),
    )?;
    if pending != 0 {
        return Err("wake acknowledgements not yet committed".into());
    }
    println!(
        "{}",
        json!({"requests":[ACTOR_TRIGGER,BUILDER_TRIGGER],"replies":[actor_reply,builder_reply],"records":4,
            "signatures_verified":true,"decrypted_locally":true,"pending_wakes":pending,"fixture":true})
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    match args.get(1).map(String::as_str) {
        Some("serve") if args.len() == 5 => {
            serve(&args[2], args[3].clone().into(), args[4].clone().into()).await
        }
        Some("send") if args.len() == 6 => {
            send(
                &args[2],
                &args[3],
                args[4].clone().into(),
                args[5].clone().into(),
            )
            .await
        }
        Some("verify") if args.len() == 5 => {
            verify(&args[2], args[3].clone().into(), args[4].clone().into()).await
        }
        _ => Err("usage: cowchat_room_fixture serve <loopback-bind> <temporary-state-dir> <verified-manifest> | send <room-origin> <gateway-origin> <state-dir> <verified-manifest> | verify <room-origin> <state-dir> <verified-manifest>".into()),
    }
}
