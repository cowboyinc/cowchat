use super::*;
use ciborium::value::Value;
use cowchat_crypto::{canonical, certificates};

impl Store {
    pub(crate) fn enroll_seated_builder(
        &self,
        room: &str,
        target: &str,
        body: &[u8],
        projection: &[u8],
        signature: &[u8],
        now: i64,
    ) -> Result<(String, String), StoreError> {
        let invalid = || StoreError::SeatedAuthorization;
        if now < 0 || body.len() > 512 * 1024 || target != format!("/rooms/{room}/builder") {
            return Err(invalid());
        }
        let input: crate::seated::OwnerEnrollment =
            serde_json::from_slice(body).map_err(|_| invalid())?;
        let decode = |s| B64.decode(s).map_err(|_| invalid());
        let identity = decode(input.identity)?;
        let identity_sig = decode(input.identity_signature)?;
        let membership = decode(input.membership)?;
        let membership_sig = decode(input.membership_signature)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (wallet,auth,key,owner_identity,owner_context): (Vec<u8>,i64,i64,Vec<u8>,Vec<u8>) = tx.query_row(
            "SELECT r.owner_wallet,r.auth_generation,r.key_generation,c.identity_certificate,c.trusted_context
             FROM seated_rooms r JOIN seated_credentials c ON c.room_id=r.room_id
             AND c.seat='0x'||lower(hex(r.owner_wallet)) AND c.auth_generation=r.auth_generation
             WHERE r.room_id=?1 LIMIT 1", [room], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).optional()?.ok_or_else(invalid)?;
        authorization::verify_member_access(&owner_context, "manage", now as u64)
            .map_err(|_| invalid())?;
        if wallet.len() != 20 || auth < 0 || key < 0 {
            return Err(invalid());
        }
        canonical::validate(&owner_identity).map_err(|_| invalid())?;
        let owner: std::collections::BTreeMap<String, Value> =
            ciborium::from_reader(owner_identity.as_slice()).map_err(|_| invalid())?;
        let chain = owner.get("chain_id").ok_or_else(invalid)?.clone();
        let text = |s: &str| Value::Text(s.into());
        let context = Value::Map(vec![
            (text("chain_id"), chain),
            (text("room"), text(room)),
            (text("room_owner"), Value::Bytes(wallet)),
            (text("membership_generation"), (auth as u64).into()),
            (text("key_generation"), (key as u64).into()),
            (text("now_ms"), (now as u64).into()),
        ]);
        let mut raw = Vec::new();
        ciborium::into_writer(&context, &mut raw).map_err(|_| invalid())?;
        let verified = certificates::verify_builder_membership(
            &identity,
            &identity_sig,
            &membership,
            &membership_sig,
            &canonical::canonicalize(&raw).map_err(|_| invalid())?,
        )
        .map_err(|_| invalid())?;
        let (id, seat, public, context, from_gen): (Vec<u8>, String, Vec<u8>, Vec<u8>, u64) =
            ciborium::from_reader(verified.as_slice()).map_err(|_| invalid())?;
        let cert = id.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let token = request::verify_received(
            projection, &public, signature, now as u64, "POST", target, body,
        )
        .map_err(|_| invalid())?;
        claim_nonce_on(&tx, &token, &public, now)?;
        let digest = Sha256::digest(serde_json::to_vec(&(
            &identity,
            &identity_sig,
            &membership,
            &membership_sig,
        ))?)
        .to_vec();
        let previous:Option<(String,Vec<u8>)>=tx.query_row("SELECT cert_id,digest FROM seated_builder_enrollments WHERE room_id=?1 AND auth_generation=?2",params![room,auth],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((prior_cert, prior_digest)) = previous {
            if prior_cert != cert || prior_digest != digest {
                return Err(StoreError::MessageConflict);
            }
            let live:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM seated_credentials WHERE room_id=?1 AND cert_id=?2 AND auth_generation=?3)",params![room,cert,auth],|r|r.get(0))?;
            if !live {
                return Err(invalid());
            }
        } else {
            let occupied:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM seated_credentials WHERE room_id=?1 AND seat=?2 AND auth_generation=?3)",params![room,seat,auth],|r|r.get(0))?;
            if occupied {
                return Err(StoreError::MessageConflict);
            }
            tx.execute("INSERT INTO seated_credentials(room_id,cert_id,seat,display_name,auth_generation,public_key,trusted_context,identity_certificate,identity_signature,membership_certificate,membership_signature,from_key_generation,enrollment_digest)
                VALUES (?1,?2,?3,'Builder',?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![room,cert,seat,auth,public,context,identity,identity_sig,membership,membership_sig,from_gen as i64,digest])?;
            tx.execute("INSERT INTO seated_builder_enrollments(room_id,auth_generation,cert_id,digest) VALUES (?1,?2,?3,?4)",params![room,auth,cert,digest])?;
        }
        tx.commit()?;
        Ok((seat, cert))
    }
}
