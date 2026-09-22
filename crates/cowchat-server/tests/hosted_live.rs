//! Explicit live acceptance client. Starts no broker, storage, or server.
#![cfg(feature = "hosted-bootstrap")]

use anyhow::{ensure, Context, Result};
use cowchat_client::CowchatClient;
use cowchat_core::{CreateRoomPayload, SendMessagePayload};
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

const BODY: &str = "Cowchat hosted River acceptance message";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepared {
    sender: String,
    observer: String,
    secret: [u8; 32],
    room: CreateRoomPayload,
    message: SendMessagePayload,
}

fn read_private(path: &Path) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.mode() & 0o077 == 0
            && meta.nlink() == 1
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.len() <= 32_768,
        "unsafe live-test file"
    );
    let mut bytes = vec![];
    file.take(32_769).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 32_768, "live-test file exceeds bound");
    Ok(bytes)
}

async fn client(url: &str, key: &str, id: &str) -> Result<CowchatClient> {
    Ok(tokio::time::timeout(
        Duration::from_secs(15),
        CowchatClient::connect_ws(url, key, id, Some(id), vec![]),
    )
    .await??)
}

async fn run() -> Result<()> {
    let mode = std::env::var("COWCHAT_HOSTED_E2E_MODE").context("set mode create or recover")?;
    ensure!(
        mode == "create" || mode == "recover",
        "invalid live-test mode"
    );
    let url = std::env::var("COWCHAT_HOSTED_E2E_URL")?;
    let parsed = reqwest::Url::parse(&url)?;
    ensure!(
        parsed.scheme() == "wss"
            || (parsed.scheme() == "ws"
                && matches!(parsed.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"))),
        "plaintext client WebSocket is restricted to local/tunneled loopback"
    );
    ensure!(
        parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.query().is_none()
            && parsed.fragment().is_none(),
        "URL must not carry credentials"
    );
    let key_file = PathBuf::from(std::env::var("COWCHAT_HOSTED_E2E_KEY_FILE")?);
    let state_file = PathBuf::from(std::env::var("COWCHAT_HOSTED_E2E_STATE")?);
    ensure!(
        key_file.is_absolute() && state_file.is_absolute(),
        "use absolute live-test paths"
    );
    let key = zeroize::Zeroizing::new(String::from_utf8(read_private(&key_file)?)?);
    let prepared = if mode == "create" {
        // Create-new prevents replacing evidence from an earlier attempt.
        let mut state = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&state_file)?;
        let run = uuid::Uuid::new_v4();
        let sender = format!("river-sender-{run}");
        let mut alice = client(&url, key.trim(), &sender).await?;
        let secret: [u8; 32] = rand::random();
        alice.set_room_secret(&secret);
        let room = CowchatClient::prepare_hosted_room(&format!("river-{run}"));
        let message = alice.prepare_message(
            room.room_id.as_deref().context("room ID")?,
            BODY,
            None,
            vec![],
            serde_json::json!({}),
        );
        let prepared = Prepared {
            sender,
            observer: format!("river-reader-{run}"),
            secret,
            room,
            message,
        };
        serde_json::to_writer(&mut state, &prepared)?;
        state.flush()?;
        state.sync_all()?;
        std::fs::File::open(state_file.parent().context("state parent")?)?.sync_all()?;
        drop(alice);
        prepared
    } else {
        serde_json::from_slice::<Prepared>(&read_private(&state_file)?)?
    };
    let mut alice = client(&url, key.trim(), &prepared.sender).await?;
    alice.set_room_secret(&prepared.secret);
    // Leave the reader locked to check that the server returns ciphertext.
    let bob = client(&url, key.trim(), &prepared.observer).await?;
    let room_id = prepared.room.room_id.as_deref().context("room ID")?;
    if mode == "create" {
        let room = alice.create_prepared_room(&prepared.room).await?;
        ensure!(room.room_id == room_id, "room ID changed");
    }
    alice.join_room(room_id).await?;
    bob.join_room(room_id).await?;
    if mode == "recover" {
        // Read BEFORE retries: replaying prepared operations must not recreate
        // state and disguise a missing recovered archive.
        let recovered = alice.get_history(room_id, 10, None).await?;
        ensure!(
            recovered.len() == 1
                && recovered[0].message_id
                    == prepared.message.message_id.clone().context("message ID")?
                && recovered[0].content == BODY
                && recovered[0].seq == 1,
            "acknowledged history not recovered"
        );
    }
    let receipt = alice.append_prepared_message(&prepared.message).await?;
    ensure!(
        receipt.content == BODY && receipt.seq == 1,
        "unexpected append receipt"
    );
    let history = bob.get_history(room_id, 10, None).await?;
    ensure!(
        history.len() == 1
            && history[0].message_id == receipt.message_id
            && history[0].content == prepared.message.content
            && cowchat_core::crypto::is_ciphertext(&history[0].content),
        "reader did not receive archived ciphertext"
    );
    let retry = alice.append_prepared_message(&prepared.message).await?;
    ensure!(
        retry.message_id == receipt.message_id && retry.seq == receipt.seq,
        "retry changed receipt"
    );
    ensure!(
        bob.room_tip(room_id).await? == 1,
        "retry advanced room history"
    );
    alice.leave_room(room_id).await?;
    drop(alice);
    let mut reconnected = client(&url, key.trim(), &prepared.sender).await?;
    reconnected.set_room_secret(&prepared.secret);
    reconnected.join_room(room_id).await?;
    let history = reconnected.get_history(room_id, 10, None).await?;
    ensure!(
        history.len() == 1 && history[0].content == BODY,
        "reconnect lost history"
    );
    println!("PASS hosted {mode}: encrypted create/send/history, exact retry, reconnect; room={room_id} message={} seq=1", receipt.message_id);
    Ok(())
}

#[tokio::test]
#[ignore = "requires an explicitly provisioned real hosted endpoint and private credentials"]
async fn hosted_live_acceptance() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(120), run()).await?
}
