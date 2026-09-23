//! Archive a prepared room-key epoch, publish/confirm/fence it, then activate
//! that exact epoch. The initial path creates the room; a successor stages only
//! the next key epoch and leaves the existing history in place.
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

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserInitialRoom {
    pub room_id: String,
    pub name: String,
    pub created_by: String,
    pub room_owner: Address,
    pub preparation: RoomKeyPreparation,
}

pub(crate) struct PreparedInitialRoom {
    room_id: String,
    created_by: String,
    preparation: RoomKeyPreparation,
    deployment: CompiledRoomDeployment,
    policy: SignedRoomKeyPolicyV1,
    grants: Vec<SignedRoomKeyGrantV1>,
    custody: Vec<u8>,
    control_root: [u8; 32],
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

fn validate_successor_policy(
    previous: &cowboy_protocol_codec::room_policy::RoomKeyPolicyV1,
    next: &cowboy_protocol_codec::room_policy::RoomKeyPolicyV1,
    room_owner: Address,
) -> Result<()> {
    previous
        .validate_successor(next)
        .map_err(|_| anyhow::anyhow!("invalid successor room policy"))?;
    ensure!(
        next.policy_epoch
            == previous
                .policy_epoch
                .checked_add(1)
                .context("room policy epoch overflow")?
            && next.active_key_epoch
                == previous
                    .active_key_epoch
                    .checked_add(1)
                    .context("room key epoch overflow")?,
        "successor must advance both room epochs exactly once"
    );
    let removed = previous
        .members
        .iter()
        .copied()
        .filter(|member| next.members.binary_search(member).is_err())
        .collect::<Vec<_>>();
    ensure!(
        previous.members.len() == next.members.len() + 1
            && removed.len() == 1
            && removed[0] != room_owner
            && next
                .members
                .iter()
                .all(|member| previous.members.binary_search(member).is_ok()),
        "successor must remove exactly one non-owner member"
    );
    Ok(())
}

fn validate_initial_grant_roster(
    policy: &cowboy_protocol_codec::room_policy::RoomKeyPolicyV1,
    room_owner: Address,
) -> Result<()> {
    ensure!(
        policy.members.binary_search(&room_owner).is_ok(),
        "initial room roster must include its owner"
    );
    Ok(())
}

/// Validate a preparation and (only when `submit` is true) reserve it in the
/// owner log. Initial activation appends `create`+`PrepareKeyEpoch`; rotation
/// appends only `PrepareKeyEpoch`. This is the sole
/// activation phase that touches `OwnerRuntime`, so the caller holds the
/// owner-write lock across just this call — never across the CBSS publication
/// ceremony in `finalize_initial_room`, which is what previously serialized
/// every send behind a multi-second activation.
///
/// `submit` is false when resuming a room whose records are already durable —
/// the publication ceremony failed after preparation and is being retried; the
/// room must already contain the exact pending preparation, so no records are
/// appended.
pub(crate) async fn stage_initial_room(
    runtime: &mut OwnerRuntime,
    input: &BrowserInitialRoom,
    submit: bool,
) -> Result<()> {
    ensure!(
        uuid::Uuid::parse_str(&input.room_id).is_ok(),
        "room id must be a UUID"
    );
    let name = crate::store::normalize_room_name(&input.name)
        .map_err(|_| anyhow::anyhow!("invalid room name"))?;
    ensure!(
        !input.created_by.is_empty() && input.created_by.len() <= 128,
        "invalid room creator"
    );
    let preparation = &input.preparation;
    // Reject a malformed or self-inconsistent preparation before any log write.
    let (_, previous, _, _, _) =
        validate_signed_preparation(preparation, input.room_owner, &input.room_id)?;
    {
        let state = runtime.state()?;
        match &previous {
            None => {
                ensure!(
                    preparation.expected_policy_hash.is_none()
                        && preparation.policy_epoch == 0
                        && preparation.key_epoch == 0,
                    "invalid initial room preparation"
                );
                match state.room(&input.room_id) {
                    None => ensure!(submit, "initial room preparation is not staged"),
                    Some(room) => ensure!(
                        !submit
                            && room.name == name
                            && room.key_state.is_none()
                            && room.key_publication.is_none()
                            && room.key_preparation.as_deref() == Some(preparation),
                        "initial room retry conflicts with current state"
                    ),
                }
            }
            Some(previous) => {
                let room = state
                    .room(&input.room_id)
                    .context("successor room does not exist")?;
                let current = room
                    .key_state
                    .as_ref()
                    .context("successor room has no active key")?;
                let publication = room
                    .key_publication
                    .as_ref()
                    .context("successor room has no active policy")?;
                ensure!(
                    room.name == name
                        && previous.policy.identity.owner == input.room_owner
                        && publication.signed_policy
                            == preparation.previous_policy.as_deref().unwrap_or_default()
                        && current.policy_hash == previous.policy.signing_hash()
                        && current.policy_epoch == previous.policy.policy_epoch
                        && current.key_epoch == previous.policy.active_key_epoch
                        && preparation.expected_policy_hash == Some(current.policy_hash)
                        && preparation.policy_epoch
                            == current
                                .policy_epoch
                                .checked_add(1)
                                .context("room policy epoch overflow")?
                        && preparation.key_epoch
                            == current
                                .key_epoch
                                .checked_add(1)
                                .context("room key epoch overflow")?,
                    "successor preparation does not extend current room state"
                );
                if submit {
                    ensure!(
                        room.key_preparation.is_none(),
                        "another room-key transition is pending"
                    );
                } else {
                    ensure!(
                        room.key_preparation.as_deref() == Some(preparation),
                        "room-key retry does not match pending preparation"
                    );
                }
            }
        }
    }
    if !submit {
        return Ok(());
    }
    let owner_id = runtime.owner_id().to_owned();
    let prepare = Command {
        owner_id: owner_id.clone(),
        command_id: preparation.command_id(),
        timestamp: chrono::Utc::now(),
        body: CommandBody::PrepareKeyEpoch {
            room_id: input.room_id.clone(),
            preparation: Box::new(preparation.clone()),
        },
    };
    let commands = if previous.is_none() {
        vec![
            Command {
                owner_id,
                command_id: format!("create:{}", input.room_id),
                timestamp: chrono::Utc::now(),
                body: CommandBody::CreateRoom {
                    room_id: input.room_id.clone(),
                    lane_id: 0,
                    name,
                    created_by: input.created_by.clone(),
                },
            },
            prepare,
        ]
    } else {
        vec![prepare]
    };
    let prepared = runtime.submit(commands).await?;
    let prepare_outcome = prepared
        .outcomes
        .last()
        .context("incomplete room-key preparation receipt")?;
    if previous.is_none() {
        accepted(
            &prepared.outcomes[0],
            |outcome| matches!(outcome, Outcome::RoomCreated { room_id, .. } if room_id == &input.room_id),
        )?;
    }
    accepted(prepare_outcome, |outcome| {
        matches!(outcome, Outcome::KeyEpochPrepared { room_id, transition_id }
            if room_id == &input.room_id && transition_id == &preparation.transition_id)
    })
}

/// Decode and cross-check the signed setup/policy/grants a preparation carries.
/// Pure and cheap, so it runs both before the log write in `stage_initial_room`
/// and again while rebuilding the publication in `finalize_initial_room`.
fn validate_signed_preparation(
    preparation: &RoomKeyPreparation,
    room_owner: Address,
    room_id: &str,
) -> Result<(
    SignedRoomSetupV1,
    Option<SignedRoomKeyPolicyV1>,
    SignedRoomKeyPolicyV1,
    Vec<SignedRoomKeyGrantV1>,
    Vec<u8>,
)> {
    let setup = SignedRoomSetupV1::decode_canonical(&decode_hex(&preparation.signed_setup)?)
        .map_err(|_| anyhow::anyhow!("invalid signed room setup"))?;
    setup
        .verify_owner_at(setup.request.issued_at_ms)
        .map_err(|_| anyhow::anyhow!("invalid signed room setup"))?;
    let previous = preparation
        .previous_policy
        .as_deref()
        .map(|value| {
            SignedRoomKeyPolicyV1::decode_canonical(&decode_hex(value)?)
                .map_err(|_| anyhow::anyhow!("invalid predecessor room policy"))
        })
        .transpose()?;
    if let Some(previous) = &previous {
        previous
            .verify_owner()
            .map_err(|_| anyhow::anyhow!("invalid predecessor room policy"))?;
    }
    let policy = SignedRoomKeyPolicyV1::decode_canonical(&decode_hex(&preparation.signed_policy)?)
        .map_err(|_| anyhow::anyhow!("invalid signed room policy"))?;
    policy
        .verify_owner()
        .map_err(|_| anyhow::anyhow!("invalid signed room policy"))?;
    let grants = preparation
        .grants
        .iter()
        .map(|grant| {
            SignedRoomKeyGrantV1::decode_cfg(decode_hex(grant)?.as_slice(), &())
                .map_err(|_| anyhow::anyhow!("invalid signed room grant"))
        })
        .collect::<Result<Vec<_>>>()?;
    let custody = decode_hex(&preparation.custody)?;
    let intent = &setup.request.intent;
    ensure!(
        setup.request.control_root == preparation.expected_control_root
            && intent.identity.owner == room_owner
            && intent.identity.room_id == room_id
            && policy.policy.identity == intent.identity
            && intent.transition_id == preparation.transition_id
            && intent.policy_epoch == preparation.policy_epoch
            && intent.key_epoch == preparation.key_epoch
            && intent.previous_policy_hash == preparation.expected_policy_hash
            && policy.policy.signing_hash() == preparation.policy_hash,
        "prepared room metadata does not match its signed publication"
    );
    intent
        .validate_predecessor(previous.as_ref().map(|value| &value.policy))
        .map_err(|_| anyhow::anyhow!("prepared room does not extend its predecessor"))?;
    if let Some(previous) = &previous {
        validate_successor_policy(&previous.policy, &policy.policy, room_owner)?;
    }
    validate_grant_matrix(&policy, &grants, room_owner)?;
    if previous.is_none() {
        validate_initial_grant_roster(&policy.policy, room_owner)?;
    }

    Ok((setup, previous, policy, grants, custody))
}

fn validate_grant_matrix(
    policy: &SignedRoomKeyPolicyV1,
    grants: &[SignedRoomKeyGrantV1],
    room_owner: Address,
) -> Result<BTreeSet<(Address, u64)>> {
    let expected_grant_count = policy
        .policy
        .members
        .len()
        .checked_mul(policy.policy.keys.len())
        .filter(|count| *count <= 1024)
        .context("room-key grant matrix exceeds publication limit")?;
    ensure!(
        grants.len() == expected_grant_count,
        "preparation must grant every retained member every room-key epoch"
    );
    let mut granted = BTreeSet::new();
    for grant in grants {
        ensure!(
            grant
                .owner_signature
                .recover_address(&grant.grant.signing_hash())
                .is_ok_and(|signer| signer == room_owner),
            "invalid room-key grant signature"
        );
        let key = policy
            .policy
            .match_grant(&grant.grant)
            .map_err(|_| anyhow::anyhow!("room-key grant does not match policy"))?;
        ensure!(
            granted.insert((grant.grant.member, key.key_epoch)),
            "preparation contains a duplicate room-key grant"
        );
    }
    let expected_grants = policy
        .policy
        .members
        .iter()
        .flat_map(|member| {
            policy
                .policy
                .keys
                .iter()
                .map(move |key| (*member, key.key_epoch))
        })
        .collect::<BTreeSet<_>>();
    ensure!(
        granted == expected_grants,
        "preparation must grant every retained member every room-key epoch"
    );
    Ok(granted)
}

fn publication(
    deployment: &CompiledRoomDeployment,
    room_owner: Address,
    room_id: &str,
    setup: SignedRoomSetupV1,
    previous_policy: Option<SignedRoomKeyPolicyV1>,
    policy: SignedRoomKeyPolicyV1,
    grants: Vec<SignedRoomKeyGrantV1>,
    custody: Vec<u8>,
) -> Result<RoomPublication> {
    RoomPublication::new(
        deployment.committee(),
        &deployment.identity(room_owner, room_id.to_owned()),
        RoomPublicationInput {
            setup,
            previous_policy,
            policy,
            custody,
            grants,
        },
    )
    .map_err(Into::into)
}

async fn confirm_publication(
    config: &Config,
    auth: &Authority,
    deployment: &CompiledRoomDeployment,
    source: &cbssd::room_control::FinalizedRoomControlSource<SystemTrustedClock>,
    publication: &RoomPublication,
) -> Result<[u8; 32]> {
    let mut reader_ctx = volume_with_access(
        config,
        auth,
        &config.control_volume,
        AccessMode::ReadOnly,
        false,
    )
    .await?;
    let confirmed = publication
        .confirm_reopened(
            deployment.committee(),
            source,
            &mut reader_ctx.volume,
            &SystemTrustedClock,
        )
        .await?;
    reader_ctx.close().await;
    Ok(confirmed.manifest_root())
}

/// Check the shared control root before the preparation is archived. A root
/// mismatch is accepted only when fresh storage already contains this exact
/// publication, which is the lost-ack retry case.
pub(crate) async fn preflight_initial_room(
    config: &Config,
    input: &BrowserInitialRoom,
) -> Result<()> {
    let (setup, previous_policy, policy, grants, custody) =
        validate_signed_preparation(&input.preparation, input.room_owner, &input.room_id)?;
    let deployment = CompiledRoomDeployment::compiled()?;
    let publication = publication(
        &deployment,
        input.room_owner,
        &input.room_id,
        setup,
        previous_policy,
        policy,
        grants,
        custody,
    )?;
    let auth = authority(config).await?;
    let reader_ctx = volume_with_access(
        config,
        &auth,
        &config.control_volume,
        AccessMode::ReadOnly,
        false,
    )
    .await?;
    if reader_ctx.volume.manifest_root().0 == input.preparation.expected_control_root {
        reader_ctx.close().await;
        return Ok(());
    }
    reader_ctx.close().await;
    let source = deployment.source(&config.worker_dir.join("room-control-finality"))?;
    confirm_publication(config, &auth, &deployment, &source, &publication)
        .await
        .context("stale room-key preparation is not already published")?;
    Ok(())
}

/// Publish and finalize the prepared key epoch against CBSS. Touches no
/// `OwnerRuntime`, so the caller runs it with the owner-write lock released —
/// its up-to-`IO_TIMEOUT` finality wait no longer blocks concurrent sends. A
/// timeout or verification error leaves the durable preparation pending, so the
/// activation can be retried through `stage_initial_room(.., submit = false)`.
pub(crate) async fn finalize_initial_room(
    config: &Config,
    input: BrowserInitialRoom,
) -> Result<PreparedInitialRoom> {
    let preparation = input.preparation;
    let (setup, previous_policy, policy, grants, custody) =
        validate_signed_preparation(&preparation, input.room_owner, &input.room_id)?;
    let deployment = CompiledRoomDeployment::compiled()?;
    let publication = publication(
        &deployment,
        input.room_owner,
        &input.room_id,
        setup.clone(),
        previous_policy,
        policy.clone(),
        grants.clone(),
        custody.clone(),
    )?;

    let auth = authority(config).await?;
    let mut writer_ctx = volume(config, &auth, &config.control_volume).await?;
    let source = deployment.source(&config.worker_dir.join("room-control-finality"))?;
    let confirmed = if writer_ctx.volume.manifest_root().0 == preparation.expected_control_root {
        let acknowledgements = deployment.attest(&publication, &setup).await?;
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

        // Wait only for this receipt to become independently finalized.
        let confirmed = tokio::time::timeout(IO_TIMEOUT, async {
            loop {
                let finalized = source.resolve().await?;
                if finalized.manifest_root() == receipt_root.0 {
                    return confirm_publication(config, &auth, &deployment, &source, &publication)
                        .await;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("room publication did not finalize before the deadline"))??;
        ensure!(
            confirmed == receipt_root.0,
            "confirmed room root differs from publication receipt"
        );
        confirmed
    } else {
        // A previous attempt may have finalized before its Cowchat commit was
        // acknowledged. Reopen and authenticate the exact bytes instead of
        // trying to publish the stale setup again.
        writer_ctx.close().await;
        confirm_publication(config, &auth, &deployment, &source, &publication)
            .await
            .context("current room publication differs from exact retry")?
    };
    deployment.fence(&policy, confirmed).await?;
    Ok(PreparedInitialRoom {
        room_id: input.room_id,
        created_by: input.created_by,
        preparation,
        deployment,
        policy,
        grants,
        custody,
        control_root: confirmed,
    })
}

pub(crate) async fn commit_initial_room(
    runtime: &mut OwnerRuntime,
    prepared: &PreparedInitialRoom,
) -> Result<()> {
    let state = RoomKeyState {
        transition_id: prepared.preparation.transition_id,
        policy_epoch: prepared.preparation.policy_epoch,
        key_epoch: prepared.preparation.key_epoch,
        policy_hash: prepared.preparation.policy_hash,
        control_root: prepared.control_root,
    };
    let commit = Command {
        owner_id: runtime.owner_id().to_owned(),
        command_id: hex::encode(prepared.preparation.transition_id),
        timestamp: chrono::Utc::now(),
        body: CommandBody::CommitKeyEpoch {
            room_id: prepared.room_id.clone(),
            expected_policy_hash: prepared.preparation.expected_policy_hash,
            state,
        },
    };
    let committed = runtime.submit(vec![commit]).await?;
    accepted(&committed.outcomes[0], |outcome| {
        matches!(outcome, Outcome::KeyEpochCommitted { room_id, key_epoch }
            if room_id == &prepared.room_id && *key_epoch == prepared.preparation.key_epoch)
    })
}

/// Native executable proof: stage the room, finalize its publication, open the
/// real CBSS key, then commit. Shares the same setup/publication/fence path as
/// the browser activation, adding the member-key open before commit.
pub async fn activate_initial_room(
    config: &Config,
    runtime: &mut OwnerRuntime,
    input: InitialRoomDemo,
) -> Result<InitialRoomProbe> {
    ensure!(
        !input.probe_text.is_empty() && input.probe_text.len() <= 4 * 1024,
        "invalid room demo probe text"
    );
    let member = member_key(&input.member_key_file)?;
    let probe_text = input.probe_text;
    let browser = BrowserInitialRoom {
        room_id: input.room_id,
        name: input.name,
        created_by: input.created_by,
        room_owner: Address::from_verifying_key(member.verifying_key()),
        preparation: input.preparation,
    };
    stage_initial_room(runtime, &browser, true).await?;
    let prepared = finalize_initial_room(config, browser).await?;
    let member_address = Address::from_verifying_key(member.verifying_key());
    let grant = prepared
        .grants
        .iter()
        .find(|grant| {
            grant.grant.member == member_address
                && prepared
                    .policy
                    .policy
                    .match_grant(&grant.grant)
                    .is_ok_and(|key| key.key_epoch == prepared.policy.policy.active_key_epoch)
        })
        .context("member key has no prepared grant for the active key epoch")?
        .clone();
    let key = prepared
        .deployment
        .open_room_key(&prepared.policy, grant, &prepared.custody, &member)
        .await?;
    commit_initial_room(runtime, &prepared).await?;
    Ok(InitialRoomProbe {
        room_id: prepared.room_id,
        agent_id: prepared.created_by,
        probe_text,
        key,
        key_epoch: prepared.preparation.key_epoch,
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
            "room-keys-probe",
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

#[cfg(test)]
mod successor_policy_tests {
    use super::*;
    use cowboy_protocol_codec::{
        room_policy::{
            RoomCustodyCommitmentV1, RoomKeyPolicyV1, RoomPolicyIdentityV1, SignedRoomKeyPolicyV1,
        },
        room_release::{RoomKeyGrantV1, SignedRoomKeyGrantV1},
        EthSignature,
    };
    use k256::ecdsa::SigningKey;

    fn address(byte: u8) -> Address {
        Address::from_bytes([byte; 20])
    }

    fn policy(
        policy_epoch: u64,
        key_epoch: u64,
        members: Vec<Address>,
        keys: Vec<RoomCustodyCommitmentV1>,
    ) -> RoomKeyPolicyV1 {
        RoomKeyPolicyV1 {
            identity: RoomPolicyIdentityV1 {
                chain_id: 1,
                chain_instance_id: [1; 32],
                service_id: [2; 32],
                owner: address(1),
                room_id: "11111111-1111-4111-8111-111111111111".into(),
            },
            policy_epoch,
            active_key_epoch: key_epoch,
            members,
            keys,
        }
    }

    fn key(epoch: u64, byte: u8) -> RoomCustodyCommitmentV1 {
        RoomCustodyCommitmentV1 {
            key_epoch: epoch,
            scope_hash: [byte; 32],
            ciphertext_hash: [byte + 1; 32],
        }
    }

    fn signed_grant(
        owner: &SigningKey,
        policy: &RoomKeyPolicyV1,
        member: Address,
        key: &RoomCustodyCommitmentV1,
    ) -> SignedRoomKeyGrantV1 {
        let grant = RoomKeyGrantV1 {
            owner: policy.identity.owner,
            member,
            scope_hash: key.scope_hash,
            ciphertext_hash: key.ciphertext_hash,
            policy_epoch: policy.policy_epoch,
        };
        SignedRoomKeyGrantV1 {
            owner_signature: EthSignature::sign(owner, &grant.signing_hash()),
            grant,
        }
    }

    #[test]
    fn successor_is_exactly_one_non_owner_removal_and_preserves_old_commitments() {
        let previous = policy(
            0,
            0,
            vec![address(1), address(2), address(3)],
            vec![key(0, 10)],
        );
        let next = policy(
            1,
            1,
            vec![address(1), address(3)],
            vec![key(0, 10), key(1, 20)],
        );
        validate_successor_policy(&previous, &next, address(1)).unwrap();

        let mut skipped = next.clone();
        skipped.policy_epoch = 2;
        assert!(validate_successor_policy(&previous, &skipped, address(1)).is_err());
        let mut owner_removed = next.clone();
        owner_removed.members = vec![address(2), address(3)];
        assert!(validate_successor_policy(&previous, &owner_removed, address(1)).is_err());
        let mut changed_old_commitment = next;
        changed_old_commitment.keys[0].ciphertext_hash = [99; 32];
        assert!(validate_successor_policy(&previous, &changed_old_commitment, address(1)).is_err());
    }

    #[test]
    fn initial_room_roster_includes_its_owner() {
        let owner = address(1);
        let with_member = policy(0, 0, vec![owner, address(2)], vec![key(0, 10)]);
        validate_initial_grant_roster(&with_member, owner).unwrap();

        let without_owner = policy(0, 0, vec![address(2)], vec![key(0, 10)]);
        assert!(validate_initial_grant_roster(&without_owner, owner).is_err());
    }

    #[test]
    fn successor_grants_are_exact_retained_member_by_every_key_matrix() {
        let owner_key = SigningKey::from_bytes((&[0x11; 32]).into()).unwrap();
        let owner = Address::from_verifying_key(owner_key.verifying_key());
        let retained = address(3);
        let removed = address(2);
        let mut members = vec![owner, retained];
        members.sort_unstable();
        let policy = policy(
            2,
            2,
            members.clone(),
            vec![key(0, 10), key(1, 20), key(2, 30)],
        );
        let mut policy = policy;
        policy.identity.owner = owner;
        let signed_policy = SignedRoomKeyPolicyV1 {
            owner_signature: EthSignature::sign(&owner_key, &policy.signing_hash()),
            policy,
        };
        let grants = signed_policy
            .policy
            .members
            .iter()
            .flat_map(|member| {
                signed_policy
                    .policy
                    .keys
                    .iter()
                    .map(|key| signed_grant(&owner_key, &signed_policy.policy, *member, key))
            })
            .collect::<Vec<_>>();

        let granted = validate_grant_matrix(&signed_policy, &grants, owner).unwrap();
        assert_eq!(granted.len(), 6);
        assert!(granted.iter().all(|(member, _)| *member != removed));

        assert!(validate_grant_matrix(&signed_policy, &grants[..grants.len() - 1], owner).is_err());

        let mut duplicate = grants.clone();
        *duplicate.last_mut().unwrap() = duplicate[0].clone();
        assert!(validate_grant_matrix(&signed_policy, &duplicate, owner).is_err());

        let mut extra = grants.clone();
        extra.push(grants[0].clone());
        assert!(validate_grant_matrix(&signed_policy, &extra, owner).is_err());

        let mut removed_member = grants.clone();
        *removed_member.last_mut().unwrap() = signed_grant(
            &owner_key,
            &signed_policy.policy,
            removed,
            signed_policy.policy.keys.last().unwrap(),
        );
        assert!(validate_grant_matrix(&signed_policy, &removed_member, owner).is_err());
    }
}
