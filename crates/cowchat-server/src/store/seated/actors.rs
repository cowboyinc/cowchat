use super::*;
use crate::actor_proof::VerifiedActorControl;
use ciborium::value::Value as V;
use cowchat_crypto::{canonical, certificates};

#[derive(PartialEq)]
struct RoomAuthority {
    owner: Vec<u8>,
    chain: u64,
    membership: u64,
    key: u64,
}
pub(crate) struct PreparedActorEnrollment {
    room: String,
    snapshot: RoomAuthority,
    actor: [u8; 20],
    identity: Vec<u8>,
    identity_sig: Vec<u8>,
    membership: Vec<u8>,
    membership_sig: Vec<u8>,
    public: Vec<u8>,
    body: Vec<u8>,
    projection: Vec<u8>,
    signature: Vec<u8>,
}
impl PreparedActorEnrollment {
    pub(crate) fn actor(&self) -> [u8; 20] {
        self.actor
    }
}
fn invalid() -> StoreError {
    StoreError::SeatedAuthorization
}
fn fields(raw: &[u8]) -> Result<std::collections::BTreeMap<String, V>, StoreError> {
    canonical::validate(raw).map_err(|_| invalid())?;
    ciborium::from_reader(raw).map_err(|_| invalid())
}
fn number(fields: &std::collections::BTreeMap<String, V>, key: &str) -> Result<u64, StoreError> {
    let Some(V::Integer(v)) = fields.get(key) else {
        return Err(invalid());
    };
    (*v).try_into().map_err(|_| invalid())
}
fn text_field(
    fields: &std::collections::BTreeMap<String, V>,
    key: &str,
) -> Result<String, StoreError> {
    let Some(V::Text(v)) = fields.get(key) else {
        return Err(invalid());
    };
    Ok(v.clone())
}
fn encode(value: V) -> Result<Vec<u8>, StoreError> {
    let mut bytes = Vec::new();
    ciborium::into_writer(&value, &mut bytes).map_err(|_| invalid())?;
    canonical::canonicalize(&bytes).map_err(|_| invalid())
}
fn map(values: Vec<(&str, V)>) -> V {
    V::Map(
        values
            .into_iter()
            .map(|(k, v)| (V::Text(k.into()), v))
            .collect(),
    )
}
fn room_authority(conn: &Connection, room: &str) -> Result<RoomAuthority, StoreError> {
    let (owner, membership, key, identity): (Vec<u8>, i64, i64, Vec<u8>) = conn
        .query_row(
            "SELECT r.owner_wallet,r.auth_generation,r.key_generation,c.identity_certificate
         FROM seated_rooms r JOIN seated_credentials c ON c.room_id=r.room_id
         AND c.seat='0x'||lower(hex(r.owner_wallet)) WHERE r.room_id=?1 LIMIT 1",
            [room],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?
        .ok_or_else(invalid)?;
    if owner.len() != 20 {
        return Err(invalid());
    }
    Ok(RoomAuthority {
        owner,
        chain: number(&fields(&identity)?, "chain_id")?,
        membership: membership.try_into().map_err(|_| invalid())?,
        key: key.try_into().map_err(|_| invalid())?,
    })
}
impl Store {
    /// Ingest a fresh finalized control proof for an already tracked actor.
    /// This only revokes superseded credentials; it grants no membership, keys,
    /// or compute authority. Production callers obtain proofs from the configured
    /// release-pinned ActorProofAuthority, never from caller-authored JSON.
    pub fn ingest_actor_control(
        &self,
        proof: &VerifiedActorControl,
        now: i64,
    ) -> Result<usize, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let tracked: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM seated_actor_floors WHERE chain_id=?1 AND actor=?2)",
            params![
                i64::try_from(proof.chain_id()).map_err(|_| invalid())?,
                proof.actor().as_slice()
            ],
            |r| r.get(0),
        )?;
        if !tracked {
            return Err(invalid());
        }
        record_actor_control_on(&tx, proof, now)?;
        let revoked = invalidate_actor_credentials_on(&tx, proof)?;
        tx.commit()?;
        Ok(revoked)
    }

    pub(crate) fn prepare_actor_enrollment(
        &self,
        room: &str,
        target: &str,
        body: &[u8],
        projection: &[u8],
        signature: &[u8],
        now: i64,
    ) -> Result<PreparedActorEnrollment, StoreError> {
        if now < 0 || body.len() > 512 * 1024 || target != format!("/rooms/{room}/actors") {
            return Err(invalid());
        }
        let input: crate::seated::OwnerEnrollment =
            serde_json::from_slice(body).map_err(|_| invalid())?;
        let decode = |s: String| B64.decode(s).map_err(|_| invalid());
        let identity = decode(input.identity)?;
        let identity_sig = decode(input.identity_signature)?;
        let membership = decode(input.membership)?;
        let membership_sig = decode(input.membership_signature)?;
        certificates::validate(certificates::IDENTITY, &identity).map_err(|_| invalid())?;
        let claims = fields(&identity)?;
        let actor_text = text_field(&claims, "address")?;
        let bare = actor_text.strip_prefix("0x").ok_or_else(invalid)?;
        if bare.len() != 40
            || !bare
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || text_field(&claims, "role")? != "actor"
        {
            return Err(invalid());
        }
        let mut actor = [0; 20];
        for (i, byte) in actor.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&bare[2 * i..2 * i + 2], 16).map_err(|_| invalid())?;
        }
        let Some(V::Bytes(public)) = claims.get("pubkey") else {
            return Err(invalid());
        };
        let public = public.clone();
        let conn = self.conn.lock().unwrap();
        let snapshot = room_authority(&conn, room)?;
        let trusted = encode(map(vec![
            ("chain_id", snapshot.chain.into()),
            ("room", V::Text(room.into())),
            ("gen", snapshot.membership.into()),
            ("now_ms", (now as u64).into()),
            ("wallet_address", V::Bytes(snapshot.owner.clone())),
            ("admin_key", V::Null),
        ]))?;
        certificates::verify(
            certificates::MEMBERSHIP,
            &membership,
            &membership_sig,
            &certificates::certificate_id(certificates::MEMBERSHIP, &membership)
                .map_err(|_| invalid())?,
            &trusted,
        )
        .map_err(|_| invalid())?;
        let member = fields(&membership)?;
        if text_field(&member, "seat")? != actor_text
            || number(&claims, "chain_id")? != snapshot.chain
        {
            return Err(invalid());
        }
        request::verify_received(
            projection, &public, signature, now as u64, "POST", target, body,
        )
        .map_err(|_| invalid())?;
        Ok(PreparedActorEnrollment {
            room: room.into(),
            snapshot,
            actor,
            identity,
            identity_sig,
            membership,
            membership_sig,
            public,
            body: body.to_vec(),
            projection: projection.to_vec(),
            signature: signature.to_vec(),
        })
    }

    pub(crate) fn install_actor_enrollment(
        &self,
        p: PreparedActorEnrollment,
        proof: VerifiedActorControl,
        now: i64,
    ) -> Result<(String, String), StoreError> {
        if now < 0
            || proof.actor() != p.actor
            || proof.chain_id() != p.snapshot.chain
            || !proof.is_fresh_at(now as u64)
        {
            return Err(invalid());
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if room_authority(&tx, &p.room)? != p.snapshot {
            return Err(invalid());
        }
        let trusted = encode(map(vec![
            ("chain_id", p.snapshot.chain.into()),
            (
                "actor",
                V::Text(format!(
                    "0x{}",
                    p.actor
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                )),
            ),
            ("controller", V::Bytes(proof.controller().to_vec())),
            (
                "certificate_commitment",
                V::Bytes(proof.commitment().to_vec()),
            ),
            (
                "authorization_generation",
                proof.authorization_generation().into(),
            ),
            ("room", V::Text(p.room.clone())),
            ("room_owner", V::Bytes(p.snapshot.owner.clone())),
            ("membership_generation", p.snapshot.membership.into()),
            ("key_generation", p.snapshot.key.into()),
            ("now_ms", (now as u64).into()),
        ]))?;
        let verified = certificates::verify_actor_membership(
            &p.identity,
            &p.identity_sig,
            &p.membership,
            &p.membership_sig,
            &trusted,
        )
        .map_err(|_| invalid())?;
        let (id, seat, public, context, from_gen): (Vec<u8>, String, Vec<u8>, Vec<u8>, u64) =
            ciborium::from_reader(verified.as_slice()).map_err(|_| invalid())?;
        let token = request::verify_received(
            &p.projection,
            &p.public,
            &p.signature,
            now as u64,
            "POST",
            &format!("/rooms/{}/actors", p.room),
            &p.body,
        )
        .map_err(|_| invalid())?;
        claim_nonce_on(&tx, &token, &p.public, now)?;
        record_actor_control_on(&tx, &proof, now)?;
        let chain = i64::try_from(proof.chain_id()).map_err(|_| invalid())?;
        let actor_gen = i64::try_from(proof.authorization_generation()).map_err(|_| invalid())?;
        let cert = id.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let digest = Sha256::digest(serde_json::to_vec(&(
            &p.identity,
            &p.identity_sig,
            &p.membership,
            &p.membership_sig,
        ))?)
        .to_vec();
        // A newer independently proven actor authorization invalidates older
        // actor credentials across rooms on this chain, never owner seats.
        invalidate_actor_credentials_on(&tx, &proof)?;
        let existing: Option<Vec<u8>> = tx
            .query_row(
                "SELECT enrollment_digest FROM seated_credentials WHERE room_id=?1 AND cert_id=?2",
                params![p.room, cert],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(original) = existing {
            if original != digest {
                return Err(StoreError::MessageConflict);
            }
        } else {
            let occupied: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM seated_credentials WHERE room_id=?1 AND seat=?2)",
                params![p.room, seat],
                |r| r.get(0),
            )?;
            if occupied {
                return Err(StoreError::MessageConflict);
            }
            tx.execute("INSERT INTO seated_credentials(room_id,cert_id,seat,display_name,auth_generation,public_key,trusted_context,
                identity_certificate,identity_signature,membership_certificate,membership_signature,from_key_generation,actor_chain_id,actor_authorization_generation,enrollment_digest)
                VALUES (?1,?2,?3,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![p.room,cert,seat,i64::try_from(p.snapshot.membership).map_err(|_|invalid())?,public,context,p.identity,p.identity_sig,p.membership,p.membership_sig,i64::try_from(from_gen).map_err(|_|invalid())?,chain,actor_gen,digest])?;
        }
        tx.commit()?;
        Ok((seat, cert))
    }
}

/// Shared by enrollment and passive proof ingestion. The caller owns the
/// Immediate transaction; a proof must never advance the floor separately
/// from invalidating the credentials it supersedes.
fn record_actor_control_on(
    conn: &Connection,
    proof: &VerifiedActorControl,
    now: i64,
) -> Result<(), StoreError> {
    if now < 0 || !proof.is_fresh_at(now as u64) {
        return Err(invalid());
    }
    let chain = i64::try_from(proof.chain_id()).map_err(|_| invalid())?;
    let height = i64::try_from(proof.height()).map_err(|_| invalid())?;
    let actor_gen = i64::try_from(proof.authorization_generation()).map_err(|_| invalid())?;
    let prior:Option<(Vec<u8>,i64,Vec<u8>,Vec<u8>,i64,Vec<u8>)>=conn.query_row(
            "SELECT chain_instance,height,block_hash,state_root,authorization_generation,commitment FROM seated_actor_floors WHERE chain_id=?1 AND actor=?2",
            params![chain,proof.actor().as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        ).optional()?;
    if let Some((instance, floor, hash, root, generation, commitment)) = prior {
        if instance != proof.chain_instance()
            || height < floor
            || actor_gen < generation
            || (height == floor && (hash != proof.block_hash() || root != proof.state_root()))
            || (actor_gen == generation && commitment != proof.commitment())
        {
            return Err(invalid());
        }
    }

    conn.execute("INSERT INTO seated_actor_floors VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
        ON CONFLICT(chain_id,actor) DO UPDATE SET height=excluded.height,block_hash=excluded.block_hash,state_root=excluded.state_root,authorization_generation=excluded.authorization_generation,commitment=excluded.commitment",
        params![chain,proof.actor().as_slice(),proof.chain_instance().as_slice(),height,proof.block_hash().as_slice(),proof.state_root().as_slice(),actor_gen,proof.commitment().as_slice()])?;
    Ok(())
}

fn invalidate_actor_credentials_on(
    conn: &Connection,
    proof: &VerifiedActorControl,
) -> Result<usize, StoreError> {
    let chain = i64::try_from(proof.chain_id()).map_err(|_| invalid())?;
    let generation = i64::try_from(proof.authorization_generation()).map_err(|_| invalid())?;
    let seat = format!(
        "0x{}",
        proof
            .actor()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    // Fence any in-flight responses before removing the credentials. Retain
    // queued immutable pointers for an explicitly authorized later repair.
    conn.execute(
        "UPDATE subscriptions SET status='failed',revision=revision+1
        WHERE subscription_id IN (SELECT s.subscription_id FROM seated_subscriptions s
        JOIN seated_credentials c ON c.room_id=s.room_id AND c.cert_id=s.cert_id
        WHERE c.actor_chain_id=?1 AND c.seat=?2 AND c.actor_authorization_generation<?3)",
        params![chain, seat, generation],
    )?;
    Ok(conn.execute("DELETE FROM seated_credentials WHERE actor_chain_id=?1 AND seat=?2 AND actor_authorization_generation<?3",
        params![chain,seat,generation])?)
}
