//! Deliberately narrow first-demo composition: create one room, archive its
//! prepared key epoch, publish/confirm/fence it, then activate that exact epoch.
//! Failure after preparation leaves the room paused for inspection.
use super::*;
use crate::room_log::{Command, CommandBody, Outcome, RoomKeyPreparation, RoomKeyState};
use cbssd::{
    room_deployment::CompiledRoomDeployment,
    room_release::publication::{RoomPublication, RoomPublicationInput, RoomPublicationWriter},
    SystemTrustedClock,
};
use commonware_codec::Decode;
use cowboy_protocol_codec::{
    room_policy::SignedRoomKeyPolicyV1, room_release::SignedRoomKeyGrantV1,
    room_setup::SignedRoomSetupV1, Address,
};
use cowchat_client::CowchatClient;
use k256::ecdsa::SigningKey;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialRoomDemo {
    pub room_id: String,
    pub name: String,
    pub created_by: String,
    pub preparation: RoomKeyPreparation,
    /// Private member signing key used only by the executable proof client.
    pub member_key_file: PathBuf,
    pub probe_text: String,
}

pub struct InitialRoomProbe {
    room_id: String,
    agent_id: String,
    probe_text: String,
    key: Zeroizing<Vec<u8>>,
    key_epoch: u64,
}

impl InitialRoomDemo {
    pub fn load(path: &Path) -> Result<Self> {
        ensure!(
            path.is_absolute(),
            "initial room input path must be absolute"
        );
        Ok(serde_json::from_slice(&read_file(
            path,
            false,
            MAX_BYTES as u64,
        )?)?)
    }
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    hex::decode(value).map_err(|_| anyhow::anyhow!("invalid canonical preparation hex"))
}

fn accepted(outcome: &Outcome, expected: impl FnOnce(&Outcome) -> bool) -> Result<()> {
    ensure!(expected(outcome), "initial room command was rejected");
    Ok(())
}

fn member_key(path: &Path) -> Result<SigningKey> {
    ensure!(path.is_absolute(), "member key path must be absolute");
    let bytes = read_file(path, true, 256)?;
    let bytes = decode_hex(std::str::from_utf8(&bytes)?.trim())?;
    SigningKey::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid member signing key"))
}

/// Execute one initial room setup before the hosted listener starts. This is
/// intentionally not a general retry/rotation API: the room must not exist and
/// its preparation must describe epoch zero with no predecessor.
pub async fn activate_initial_room(
    config: &Config,
    runtime: &mut OwnerRuntime,
    input: InitialRoomDemo,
) -> Result<InitialRoomProbe> {
    ensure!(
        uuid::Uuid::parse_str(&input.room_id).is_ok(),
        "initial room id must be a UUID"
    );
    let name = crate::store::normalize_room_name(&input.name)
        .map_err(|_| anyhow::anyhow!("invalid initial room name"))?;
    ensure!(
        !input.created_by.is_empty() && input.created_by.len() <= 128,
        "invalid initial room creator"
    );
    ensure!(
        !input.probe_text.is_empty() && input.probe_text.len() <= 4 * 1024,
        "invalid room demo probe text"
    );
    ensure!(
        runtime.state()?.room(&input.room_id).is_none(),
        "initial room already exists; automatic retry is not supported"
    );

    let preparation = input.preparation;
    ensure!(
        preparation.expected_policy_hash.is_none()
            && preparation.previous_policy.is_none()
            && preparation.policy_epoch == 0
            && preparation.key_epoch == 0,
        "first demo supports only initial room provisioning"
    );
    let setup = SignedRoomSetupV1::decode_canonical(&decode_hex(&preparation.signed_setup)?)
        .map_err(|_| anyhow::anyhow!("invalid signed room setup"))?;
    let policy = SignedRoomKeyPolicyV1::decode_canonical(&decode_hex(&preparation.signed_policy)?)
        .map_err(|_| anyhow::anyhow!("invalid signed room policy"))?;
    let grants = preparation
        .grants
        .iter()
        .map(|grant| {
            SignedRoomKeyGrantV1::decode_cfg(decode_hex(grant)?.as_slice(), &())
                .map_err(|_| anyhow::anyhow!("invalid signed room grant"))
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        grants.len() == 1,
        "first demo requires exactly one room-key member grant"
    );
    let custody = decode_hex(&preparation.custody)?;
    let intent = &setup.request.intent;
    ensure!(
        setup.request.control_root == preparation.expected_control_root
            && intent.transition_id == preparation.transition_id
            && intent.policy_epoch == preparation.policy_epoch
            && intent.key_epoch == preparation.key_epoch
            && intent.previous_policy_hash == preparation.expected_policy_hash
            && policy.policy.signing_hash() == preparation.policy_hash,
        "prepared room metadata does not match its signed publication"
    );
    let member = member_key(&input.member_key_file)?;
    ensure!(
        Address::from_verifying_key(member.verifying_key()) == grants[0].grant.member,
        "member key does not match the prepared room grant"
    );

    let deployment = CompiledRoomDeployment::compiled()?;
    let expected_identity = deployment.identity(intent.identity.owner, input.room_id.clone());
    let publication = RoomPublication::new(
        deployment.committee(),
        &expected_identity,
        RoomPublicationInput {
            setup: setup.clone(),
            previous_policy: None,
            policy: policy.clone(),
            custody: custody.clone(),
            grants: grants.clone(),
        },
    )?;

    // Acquire the exact owner-write root before archiving the preparation. No
    // storage write occurs if the client signed a stale/wrong predecessor.
    let auth = authority(config).await?;
    let mut writer_ctx = volume(config, &auth, &config.control_volume).await?;
    ensure!(
        writer_ctx.volume.manifest_root().0 == preparation.expected_control_root,
        "initial room setup does not bind the current control root"
    );

    let owner_id = runtime.owner_id().to_owned();
    let create = Command {
        owner_id: owner_id.clone(),
        command_id: format!("create:{}", input.room_id),
        timestamp: chrono::Utc::now(),
        body: CommandBody::CreateRoom {
            room_id: input.room_id.clone(),
            lane_id: 0,
            name,
            created_by: input.created_by.clone(),
        },
    };
    let prepare = Command {
        owner_id: owner_id.clone(),
        command_id: preparation.command_id(),
        timestamp: chrono::Utc::now(),
        body: CommandBody::PrepareKeyEpoch {
            room_id: input.room_id.clone(),
            preparation: Box::new(preparation.clone()),
        },
    };
    let prepared = runtime.submit(vec![create, prepare]).await?;
    ensure!(
        prepared.outcomes.len() == 2,
        "incomplete initial room receipt"
    );
    accepted(
        &prepared.outcomes[0],
        |outcome| matches!(outcome, Outcome::RoomCreated { room_id, .. } if room_id == &input.room_id),
    )?;
    accepted(&prepared.outcomes[1], |outcome| {
        matches!(outcome, Outcome::KeyEpochPrepared { room_id, transition_id }
            if room_id == &input.room_id && transition_id == &preparation.transition_id)
    })?;

    let acknowledgements = deployment.attest(&publication, &setup).await?;
    let source = deployment.source(&config.worker_dir.join("room-control-finality"))?;
    let receipt_root = {
        let mut writer = RoomPublicationWriter {
            committee: deployment.committee(),
            source: &source,
            volume: &mut writer_ctx.volume,
            registry: writer_ctx.registry.as_ref(),
            clock: SystemTrustedClock,
        };
        writer
            .publish(&publication, &setup, &acknowledgements)
            .await?
    };
    writer_ctx.close().await;

    // Wait only for this receipt to become independently finalized. Timeout or
    // any verification error leaves the durable preparation pending.
    let confirmed = tokio::time::timeout(IO_TIMEOUT, async {
        loop {
            let finalized = source.resolve().await?;
            if finalized.manifest_root() == receipt_root.0 {
                let mut reader_ctx = volume_with_access(
                    config,
                    &auth,
                    &config.control_volume,
                    AccessMode::ReadOnly,
                    false,
                )
                .await?;
                let confirmed = publication
                    .confirm_reopened(
                        deployment.committee(),
                        &source,
                        &mut reader_ctx.volume,
                        &SystemTrustedClock,
                    )
                    .await?;
                reader_ctx.close().await;
                return Ok::<_, anyhow::Error>(confirmed);
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("room publication did not finalize before the deadline"))??;
    ensure!(
        confirmed.manifest_root() == receipt_root.0,
        "confirmed room root differs from publication receipt"
    );
    deployment.fence(&policy, confirmed.manifest_root()).await?;
    // Prove the freshly admitted member can actually recover this custody
    // object before the writer records the epoch as active.
    let key = deployment
        .open_room_key(&policy, grants[0].clone(), &custody, &member)
        .await?;

    let state = RoomKeyState {
        transition_id: preparation.transition_id,
        policy_epoch: preparation.policy_epoch,
        key_epoch: preparation.key_epoch,
        policy_hash: preparation.policy_hash,
        control_root: confirmed.manifest_root(),
    };
    let commit = Command {
        owner_id,
        command_id: hex::encode(preparation.transition_id),
        timestamp: chrono::Utc::now(),
        body: CommandBody::CommitKeyEpoch {
            room_id: input.room_id.clone(),
            expected_policy_hash: preparation.expected_policy_hash,
            state,
        },
    };
    let committed = runtime.submit(vec![commit]).await?;
    accepted(&committed.outcomes[0], |outcome| {
        matches!(outcome, Outcome::KeyEpochCommitted { room_id, key_epoch }
            if room_id == &input.room_id && *key_epoch == preparation.key_epoch)
    })?;
    Ok(InitialRoomProbe {
        room_id: input.room_id,
        agent_id: input.created_by,
        probe_text: input.probe_text,
        key,
        key_epoch: preparation.key_epoch,
    })
}

/// Prove the activated room is usable: the member opens the real CBSS key,
/// sends one contextual ciphertext through the normal hosted socket, replays
/// it from history, and decrypts the stored bytes locally.
pub async fn probe_initial_room(config: &Config, probe: InitialRoomProbe) -> Result<(String, i64)> {
    let key: &[u8; 32] = probe
        .key
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("CBSS returned an invalid room key"))?;

    let api_key = read_file(&config.api_key_file, true, 4096)?;
    let api_key = std::str::from_utf8(&api_key)?.trim();
    let socket = config.worker_dir.join("server.sock");
    let deadline = Instant::now() + Duration::from_secs(10);
    let client = loop {
        match CowchatClient::connect_uds(
            &socket,
            api_key,
            "room-key-demo",
            Some(&probe.agent_id),
            vec![],
        )
        .await
        {
            Ok(client) => break client,
            Err(error) if Instant::now() < deadline => {
                log::debug!("Waiting for hosted demo socket: {error}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(error) => return Err(error.into()),
        }
    };
    client.join_room(&probe.room_id).await?;
    let message_id = uuid::Uuid::new_v4().to_string();
    let context = cowchat_core::room_crypto::Context {
        room_id: &probe.room_id,
        key_epoch: probe.key_epoch,
        message_id: &message_id,
    };
    let payload = CowchatClient::prepare_room_key_message(
        key,
        &context,
        &probe.probe_text,
        None,
        vec![],
        serde_json::json!({"demo":"cbss-room-key"}),
    )?;
    client.append_prepared_message(&payload).await?;
    let history = client.get_history(&probe.room_id, 20, None).await?;
    let stored = history
        .iter()
        .find(|message| message.message_id == message_id)
        .context("room demo message missing from replayed history")?;
    ensure!(
        stored.key_epoch.as_deref() == Some(probe.key_epoch.to_string().as_str()),
        "replayed room message has the wrong key epoch"
    );
    let plaintext = cowchat_core::room_crypto::decrypt(key, &context, &stored.content)?;
    ensure!(
        plaintext == probe.probe_text,
        "replayed room message did not decrypt to the submitted text"
    );
    Ok((message_id, stored.seq))
}
