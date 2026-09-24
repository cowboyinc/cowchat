use super::*;
use cbfs_cli::token_refresh::owner_token_minter;
use cbfs_registry_proto::OpenVolumeResponse;

fn sample_identity() -> cbfs_auth::CbfsIdentity {
    let signing_key = SigningKey::from_bytes(&[5u8; 32]);
    let cbfs_public_key = signing_key.verifying_key().to_bytes();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    cbfs_auth::CbfsIdentity {
        rpc_base_url: "https://rpc.example".to_string(),
        delegation: cbfs_registry_proto::DelegationCert {
            wallet_address: cbfs_registry_proto::Address::from_low_u64(9),
            cbfs_public_key,
            scope: "cbfs".to_string(),
            service_class: cbfs_registry_proto::ServiceDelegationClass::MountCapableV1,
            chain_id: 0,
            network: "local".to_string(),
            aud: cbfs_registry_proto::OWNER_CAP_TOKEN_AUDIENCE.to_string(),
            expires_at_ms: now_ms + 60_000,
            signature: vec![0u8; 65],
        },
        ras_delegation: cbfs_registry_proto::RasOwnerDelegation {
            wallet_address: cbfs_registry_proto::Address::from_low_u64(9),
            delegated_pubkey: cbfs_public_key,
            scopes: vec!["cbfs".to_string()],
            service_class: cbfs_registry_proto::ServiceDelegationClass::MountCapableV1,
            chain_id: 0,
            network: "local".to_string(),
            aud: cbfs_registry_proto::OWNER_CAP_TOKEN_AUDIENCE.to_string(),
            valid_from_epoch: 0,
            valid_until_epoch: u64::MAX,
            signature: [0u8; 65],
        },
        cbfs_signing_key: signing_key,
    }
}

fn sample_open() -> OpenVolumeResponse {
    OpenVolumeResponse {
        volume_id: [9u8; 32],
        volume_name: "mounted".to_string(),
        owner_address: cbfs_registry_proto::Address::from_low_u64(9),
        status: cbfs_registry_proto::VolumeStatus::Active,
        visibility: cbfs_registry_proto::Visibility::Private,
        erasure_k: 3,
        erasure_m: 2,
        relay_endpoints: vec![],
        authoritative_root: [0u8; 32],
        effective_size_bytes: 0,
        balance_reserved: 0,
        max_owner_token_ttl_secs: 30,
        clock_skew_tolerance_secs: 60,
        wrapped_dek: None,
        owner_wrapped_dek: None,
        grantee_wrapped_dek: None,
    }
}

fn refreshed(access_mode: AccessMode) -> cbfs_registry_proto::OwnerCapTokenV1 {
    let mint = owner_token_minter(
        cbfs_auth::CowboyClient::new(sample_identity()),
        sample_open(),
        access_mode,
        5,
    );
    let minted = mint().expect("minting a refresh token");
    match cbfs_registry_proto::decode_cap_token(&minted.token_bytes)
        .expect("decoding the refreshed token")
    {
        cbfs_registry_proto::DecodedCapToken::Owner(token) => token,
        other => panic!("expected an owner cap token, got {other:?}"),
    }
}

#[test]
fn refreshed_owner_tokens_preserve_every_access_mode() {
    for mode in [
        AccessMode::ReadOnly,
        AccessMode::WriteOnly,
        AccessMode::ReadWrite,
    ] {
        let token = refreshed(mode);
        assert_eq!(
            token.access_mode,
            cbfs_registry_proto::owner_cap_access_mode(mode)
        );
        assert!(token.valid_until_ms > now_ms().unwrap());
        assert!(token.mount);
        assert_eq!(token.sync_interval, 5);
    }
}

#[test]
fn remint_preserves_authority_and_signs_fresh_nonce_and_lifetime() {
    let admin = SigningKey::from_bytes(&[1; 32]);
    let old = wire::StreamGrantV2 {
        version: wire::CBQS_VERSION_V2,
        chain_instance_id: [2; 32],
        stream_id: [3; 32],
        authorization_generation: 4,
        policy_epoch: 5,
        grant_nonce: [6; 32],
        holder_signing_key: wire::SigningPublicKeyV2 {
            algorithm: wire::SigningKeyAlgorithmV2::Ed25519,
            key_bytes: [7; 32],
        },
        verbs: wire::CBQS_V2_VERB_APPEND | wire::CBQS_V2_VERB_REPLAY,
        lane_scope: wire::LaneScopeV2::Exact(8),
        group_scope: wire::GroupScopeV2::Any,
        not_before_ms: 1000,
        expires_at_ms: 121000,
        max_message_bytes: 1024,
        max_append_bytes_per_sec: 2048,
        signature: wire::CbqsSignatureV2([0; 64]),
    };
    let renewed = remint_grant(old.clone(), &admin, 81000, 201000);
    assert_ne!(renewed.grant_nonce, old.grant_nonce);
    let again = remint_grant(old.clone(), &admin, 81000, 201000);
    assert_ne!(again.grant_nonce, renewed.grant_nonce);
    assert_eq!(renewed.not_before_ms, 81000);
    assert_eq!(renewed.expires_at_ms, 201000);
    admin
        .verifying_key()
        .verify_strict(
            &cowboy_protocol_codec::keccak256(&wire::stream_grant_signing_bytes_v2(&renewed)),
            &ed25519_dalek::Signature::from_bytes(&renewed.signature.0),
        )
        .unwrap();
    let mut expected = old;
    expected.grant_nonce = renewed.grant_nonce;
    expected.not_before_ms = renewed.not_before_ms;
    expected.expires_at_ms = renewed.expires_at_ms;
    expected.signature = renewed.signature;
    assert_eq!(renewed, expected);
}

/// Never attaches: these tests cover only the horizon arithmetic.
struct Unused;

impl Issue for Unused {
    async fn attach(
        &mut self,
        _: u64,
        _: u64,
    ) -> Result<(SessionV2, wire::StreamGrantV2), Failure> {
        unreachable!()
    }
}

pub(super) fn token(expires_at_ms: u64) -> cbfs_cli::token_refresh::OwnerTokenRefresh {
    cbfs_cli::token_refresh::OwnerTokenRefresh {
        mint: Box::new(move || {
            Ok(cbfs_cli::token_refresh::MintedToken {
                token_bytes: vec![1],
                expires_at_ms,
            })
        }),
        http_config: cbfs_hooks::cowboy::CowboyHttpConfig::new("http://127.0.0.1:1"),
        initial_expires_at_ms: expires_at_ms,
    }
}

fn remaining(horizon: Instant) -> u64 {
    horizon
        .saturating_duration_since(Instant::now())
        .as_millis() as u64
}

#[test]
fn horizon_is_the_earliest_credential_less_the_safety_margin() {
    let now = now_ms().unwrap();
    let hour = 3_600_000;
    for (grant, delegation, tokens, earliest) in [
        (
            now + hour,
            now + 2 * hour,
            [now + 3 * hour, now + 3 * hour],
            hour,
        ),
        (
            now + 3 * hour,
            now + hour,
            [now + 3 * hour, now + 3 * hour],
            hour,
        ),
        (
            now + 3 * hour,
            now + 3 * hour,
            [now + 3 * hour, now + hour],
            hour,
        ),
    ] {
        let renewal =
            Renewal::new(Unused, hour, 1, grant, delegation, tokens.map(token).into()).unwrap();
        let left = remaining(renewal.horizon());
        assert!(left <= earliest - SAFETY_MS && left + 1_000 >= earliest - SAFETY_MS);
    }
}

#[test]
fn horizon_never_exceeds_the_delegation() {
    let now = now_ms().unwrap();
    let mut renewal = Renewal::new(
        Unused,
        86_400_000,
        1,
        now + 60_000,
        now + 120_000,
        vec![token(now + 86_400_000)],
    )
    .unwrap();
    renewal.grant_expiry = now + 86_400_000;
    assert!(renewal.new_horizon().unwrap() <= renewal.delegation_horizon);
    assert!(remaining(renewal.new_horizon().unwrap()) <= 120_000 - SAFETY_MS);
}

#[test]
fn credentials_inside_the_safety_margin_refuse_to_start() {
    let now = now_ms().unwrap();
    let far = now + 3_600_000;
    for (grant, delegation, expiry) in [
        (now + SAFETY_MS - 1, far, far),
        (far, now + SAFETY_MS - 1, far),
        (far, far, now + SAFETY_MS - 1),
    ] {
        assert!(
            Renewal::new(Unused, 3_600_000, 1, grant, delegation, vec![token(expiry)]).is_err()
        );
    }
}
