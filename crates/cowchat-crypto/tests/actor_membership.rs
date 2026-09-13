use ciborium::value::Value as V;
use cowchat_crypto::{canonical, certificates as cert};
use k256::ecdsa::SigningKey;
use sha3::{Digest, Keccak256};

fn text(s: &str) -> V {
    V::Text(s.into())
}
fn map(fields: Vec<(&str, V)>) -> V {
    V::Map(fields.into_iter().map(|(k, v)| (text(k), v)).collect())
}
fn encode(value: &V) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(value, &mut out).unwrap();
    canonical::canonicalize(&out).unwrap()
}
fn replace(value: &V, key: &str, replacement: V) -> V {
    let V::Map(mut fields) = value.clone() else {
        panic!()
    };
    fields.iter_mut().find(|(k, _)| k == &text(key)).unwrap().1 = replacement;
    V::Map(fields)
}
fn wallet(key: &SigningKey) -> Vec<u8> {
    let public = key.verifying_key().to_encoded_point(false);
    Keccak256::digest(&public.as_bytes()[1..])[12..].to_vec()
}
fn sign(kind: u32, value: &V, key: &SigningKey) -> Vec<u8> {
    let signed = cert::signing_bytes(kind, &encode(value)).unwrap();
    let (signature, recovery) = key
        .sign_prehash_recoverable(&Keccak256::digest(signed))
        .unwrap();
    let mut out = signature.to_bytes().to_vec();
    out.push(recovery.to_byte());
    out
}

#[test]
fn actor_identity_requires_controller_commitment_and_independent_room_owner_membership() {
    let controller = SigningKey::from_slice(&[7; 32]).unwrap();
    let owner = SigningKey::from_slice(&[8; 32]).unwrap();
    let actor = "0x1111111111111111111111111111111111111111";
    let identity = map(vec![
        ("v", 3u64.into()),
        ("chain_id", 42u64.into()),
        ("address", text(actor)),
        ("pubkey", V::Bytes(vec![1; 32])),
        ("enc_pubkey", V::Bytes(vec![2; 32])),
        ("role", text("actor")),
        ("aud", text("cowchat")),
        ("gen", 7u64.into()),
        ("expires_at", V::Null),
    ]);
    let member = map(vec![
        ("v", 3u64.into()),
        ("chain_id", 42u64.into()),
        ("room", text("test-room")),
        ("seat", text(actor)),
        ("rights", V::Array(vec![text("read"), text("write")])),
        ("door_kind", V::Null),
        ("bound_sender", V::Null),
        ("from_gen", 2u64.into()),
        ("gen", 3u64.into()),
        ("signer_kind", text("wallet")),
        ("signer_key", V::Null),
        ("expires_at", 1000u64.into()),
    ]);
    // This is a trusted-state fixture, not a chain proof. Actor authorization,
    // room membership, and encryption-key generations deliberately differ.
    let trusted = map(vec![
        ("chain_id", 42u64.into()),
        ("actor", text(actor)),
        ("controller", V::Bytes(wallet(&controller))),
        (
            "certificate_commitment",
            V::Bytes(cert::certificate_id(cert::IDENTITY, &encode(&identity)).unwrap()),
        ),
        ("authorization_generation", 7u64.into()),
        ("room", text("test-room")),
        ("room_owner", V::Bytes(wallet(&owner))),
        ("membership_generation", 3u64.into()),
        ("key_generation", 4u64.into()),
        ("now_ms", 900u64.into()),
    ]);
    let verify = |identity: &V,
                  identity_signer: &SigningKey,
                  member: &V,
                  member_signer: &SigningKey,
                  trusted: &V| {
        cert::verify_actor_membership(
            &encode(identity),
            &sign(cert::IDENTITY, identity, identity_signer),
            &encode(member),
            &sign(cert::MEMBERSHIP, member, member_signer),
            &encode(trusted),
        )
    };
    let result = verify(&identity, &controller, &member, &owner, &trusted).unwrap();
    let V::Array(fields) = ciborium::from_reader(result.as_slice()).unwrap() else {
        panic!()
    };
    assert_eq!(fields[1], text(actor));
    assert_eq!(fields[4], 2u64.into());
    let V::Bytes(context) = &fields[3] else {
        panic!()
    };
    cowchat_crypto::authorization::verify_member_access(context, "read", 900).unwrap();
    assert!(cowchat_crypto::authorization::verify_member_access(context, "manage", 900).is_err());
    assert!(verify(&identity, &owner, &member, &owner, &trusted).is_err());
    assert!(verify(&identity, &controller, &member, &controller, &trusted).is_err());
    for (key, replacement) in [
        ("chain_id", 43u64.into()),
        ("actor", text("0x2222222222222222222222222222222222222222")),
        ("controller", V::Bytes(wallet(&owner))),
        ("certificate_commitment", V::Bytes(vec![0; 32])),
        ("authorization_generation", 8u64.into()),
        ("room", text("foreign-room")),
        ("room_owner", V::Bytes(wallet(&controller))),
        ("membership_generation", 4u64.into()),
        ("key_generation", 1u64.into()),
        ("now_ms", 1001u64.into()),
    ] {
        assert!(
            verify(
                &identity,
                &controller,
                &member,
                &owner,
                &replace(&trusted, key, replacement)
            )
            .is_err(),
            "{key}"
        );
    }
    for (key, replacement) in [
        ("seat", text("0x2222222222222222222222222222222222222222")),
        ("room", text("foreign-room")),
        ("from_gen", 5u64.into()),
    ] {
        assert!(
            verify(
                &identity,
                &controller,
                &replace(&member, key, replacement),
                &owner,
                &trusted
            )
            .is_err(),
            "{key}"
        );
    }
    let changed_key = replace(&identity, "pubkey", V::Bytes(vec![3; 32]));
    assert!(verify(&changed_key, &controller, &member, &owner, &trusted).is_err());
    let door = replace(&identity, "role", text("door"));
    let door_context = replace(
        &trusted,
        "certificate_commitment",
        V::Bytes(cert::certificate_id(cert::IDENTITY, &encode(&door)).unwrap()),
    );
    assert!(verify(&door, &controller, &member, &owner, &door_context).is_err());
}
