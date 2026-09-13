//! Uses real threshold-finality and QMDB verification on a synthetic local
//! chain. This is not evidence of a deployed actor or a Canyon trust anchor.
use super::*;
use commonware_codec::Encode;
use commonware_consensus::{
    simplex::{
        scheme::bls12381_threshold,
        types::{Finalize, Proposal},
    },
    types::{Epoch, Round, View},
};
use commonware_cryptography::{
    bls12381::primitives::variant::MinSig, certificate::mocks::Fixture, ed25519,
    sha256::Digest as Hash, Signer as _,
};
use commonware_parallel::Sequential;
use cowboy_protocol_codec::*;
use cowboy_protocol_light_client::{
    actor_storage_state_key_v1, fixture_chunk_digest, fixture_operation_leaf_digest,
    fixture_two_leaf_root,
};
use rand::{rngs::StdRng, SeedableRng};
type Scheme = bls12381_threshold::vrf::Scheme<ed25519::PublicKey, MinSig>;
type Finalization = commonware_consensus::simplex::types::Finalization<Scheme, Hash>;

fn operation(actor: Address, key: &[u8], value: &[u8], next: &[u8]) -> Vec<u8> {
    let mut out = vec![0xd2];
    out.extend_from_slice(&actor_storage_state_key_v1(actor, key).unwrap());
    out.push(3);
    let mut len = value.len() as u64;
    while len >= 128 {
        out.push((len as u8 & 127) | 128);
        len >>= 7;
    }
    out.push(len as u8);
    out.extend_from_slice(value);
    out.extend_from_slice(next);
    out
}

pub(crate) fn proof(timestamp: u64, value: Vec<u8>) -> (Vec<u8>, Vec<u8>, [u8; 20]) {
    let actor = Address::from_low_u64(9);
    let key = ACTOR_CONTROL_KEY;
    let next = actor_storage_state_key_v1(actor, b"z").unwrap();
    let first = operation(actor, key, &value, &next);
    let second = operation(
        actor,
        b"z",
        b"next",
        &actor_storage_state_key_v1(actor, b"zz").unwrap(),
    );
    let mut chunk = [0; 32];
    chunk[0] = 3;
    let ops_root = [0x99; 32];
    let root = fixture_two_leaf_root(&first, &second, &chunk, &ops_root);
    let state_proof = QmdbStateProofV1 {
        version: 1,
        loc: 0,
        chunk,
        mmr_leaves: 2,
        inactive_peaks: 0,
        digests: vec![fixture_operation_leaf_digest(1, &second)],
        partial_chunk_digest: Some(fixture_chunk_digest(&chunk)),
        ops_root,
        encoded_operation: first,
    };
    let mut rng = StdRng::seed_from_u64(7);
    let Fixture { schemes, .. } =
        bls12381_threshold::vrf::fixture::<MinSig, _>(&mut rng, b"_COWBOY", 4);
    let parent = [0x44; 32];
    let context = commonware_consensus::simplex::types::Context {
        round: Round::new(Epoch::zero(), View::new(10)),
        leader: ed25519::PrivateKey::from_seed(9).public_key(),
        parent: (View::new(9), Hash::from(parent)),
    };
    let header = FinalizedHeaderV1 {
        version: COWBOY_BLOCK_VERSION_V2,
        consensus_context: context.encode().to_vec(),
        parent,
        height: 10,
        timestamp,
        transaction_digests: vec![],
        tx_root: [0x31; 32],
        state_root: root,
        receipt_root: [0x32; 32],
        extra_data: vec![],
        consumed_seeds: vec![],
        inline_timer_manifest_committed: true,
        inline_timer_ids: vec![],
        presence_input: vec![],
    };
    let proposal = Proposal::new(
        context.round,
        context.parent.0,
        Hash::from(finalized_header_block_hash_v1(&header)),
    );
    let finalizes = schemes
        .iter()
        .map(|s| Finalize::sign(s, proposal.clone()).unwrap())
        .collect::<Vec<_>>();
    let finalization = Finalization::from_finalizes(&schemes[0], &finalizes, &Sequential)
        .unwrap()
        .encode()
        .to_vec();
    let checkpoint = TrustedCheckpointV1 {
        version: 1,
        chain_id: 42,
        chain_instance_id: [0x11; 32],
        consensus_epoch: 0,
        height: 9,
        block_hash: parent,
        state_root: [0x55; 32],
        consensus_identity: schemes[0].identity().encode().as_ref().try_into().unwrap(),
    };
    let bundle = FinalizedStateProofBundleV1 {
        version: 1,
        target_finalization: finalization,
        headers: vec![header],
        claims: vec![ActorStorageClaimV1 {
            actor,
            logical_key: key.to_vec(),
            result: ActorStorageClaimResultV1::Present {
                value,
                proof: state_proof,
            },
        }],
    };
    (
        checkpoint.encode().to_vec(),
        bundle.encode().to_vec(),
        *actor.as_bytes(),
    )
}

pub(crate) fn authority(checkpoint: Vec<u8>, endpoint: String) -> ActorProofAuthority {
    ActorProofAuthority {
        checkpoint,
        checkpoint_height: 9,
        endpoint: endpoint.parse().unwrap(),
        client: reqwest::Client::new(),
    }
}

fn control() -> Vec<u8> {
    use ciborium::value::Value as V;
    let value = V::Map(vec![
        (V::Text("controller".into()), V::Bytes(vec![7; 20])),
        (
            V::Text("certificate_commitment".into()),
            V::Bytes(vec![8; 32]),
        ),
        (V::Text("authorization_generation".into()), 3u64.into()),
    ]);
    let mut raw = Vec::new();
    ciborium::into_writer(&value, &mut raw).unwrap();
    canonical::canonicalize(&raw).unwrap()
}

#[tokio::test]
async fn actor_control_fetch_verifies_actual_finalized_proof_and_freshness() {
    use axum::{routing::post, Json, Router};
    let now = chrono::Utc::now().timestamp_millis() as u64;
    let (checkpoint, bundle, actor) = proof(now, control());
    let response = bundle.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let receiver=Router::new().route("/proof/finalized-state",post(move |Json(body):Json<serde_json::Value>| {
        let response=response.clone();async move {
            assert_eq!(body,serde_json::json!({"checkpoint_height":9,"claims":[{"actor":format!("0x{}",hex(&actor)),"logical_key_hex":format!("0x{}",hex(ACTOR_CONTROL_KEY))}]}));response
        }
    }));
    let server = tokio::spawn(async move { axum::serve(listener, receiver).await.unwrap() });
    let authority = ActorProofAuthority {
        checkpoint,
        checkpoint_height: 9,
        endpoint: format!("http://{addr}/proof/finalized-state")
            .parse()
            .unwrap(),
        client: reqwest::Client::new(),
    };
    let verified = authority.fetch(actor).await.unwrap();
    assert_eq!(verified.actor(), actor);
    assert_eq!(verified.controller(), [7; 20]);
    assert_eq!(verified.commitment(), [8; 32]);
    assert_eq!(verified.authorization_generation(), 3);
    assert_eq!(verified.height(), 10);
    assert_eq!(verified.chain_id(), 42);
    assert!(authority.verify([0; 20], &bundle, now).is_err());
    assert!(authority
        .verify(actor, &bundle, now + MAX_AGE_MS + 1)
        .is_err());
    assert!(authority
        .verify(actor, &bundle, now - MAX_FUTURE_SKEW_MS - 1)
        .is_err());
    let mut changed = bundle.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    assert!(authority.verify(actor, &changed, now).is_err());
    let (_, bad_control, _) = proof(now, b"not canonical control".to_vec());
    assert!(authority.verify(actor, &bad_control, now).is_err());
    server.abort();
}

/// Test fixture still runs the real threshold-finality and QMDB verifier.
pub(crate) fn verified_control(timestamp: u64, value: Vec<u8>) -> VerifiedActorControl {
    let (checkpoint, bytes, actor) = proof(timestamp, value);
    authority(
        checkpoint,
        "http://127.0.0.1:1/proof/finalized-state".into(),
    )
    .verify(actor, &bytes, timestamp)
    .unwrap()
}
