use super::*;
use ciborium::value::Value;
use cowchat_crypto::{canonical, certificates, key_envelope};

#[derive(serde::Deserialize)]
struct Context {
    chain_id: u64,
    room: String,
    seat: String,
    role: String,
    cert: String,
}
struct Member {
    public: Vec<u8>,
    identity: Vec<u8>,
    context: Context,
    from_gen: i64,
}
fn member_on(
    conn: &Connection,
    room: &str,
    cert: &str,
    right: &str,
    now: u64,
) -> Result<Member, StoreError> {
    let invalid = || StoreError::SeatedAuthorization;
    let (public, identity, raw, from_gen): (Vec<u8>, Vec<u8>, Vec<u8>, i64) = conn
        .query_row(
            "SELECT c.public_key,c.identity_certificate,c.trusted_context,c.from_key_generation
         FROM seated_credentials c JOIN seated_rooms r ON r.room_id=c.room_id
         WHERE c.room_id=?1 AND c.cert_id=?2 AND c.auth_generation=r.auth_generation",
            params![room, cert],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or_else(invalid)?;
    authorization::verify_member_access(&raw, right, now).map_err(|_| invalid())?;
    let context: Context = ciborium::from_reader(raw.as_slice()).map_err(|_| invalid())?;
    if context.room != room || context.cert != cert || from_gen < 0 {
        return Err(invalid());
    }
    Ok(Member {
        public,
        identity,
        context,
        from_gen,
    })
}

/// Compute the expected portable scope from service-owned, authenticated state.
/// This deliberately supports only the current room owner as key publisher.
/// A delegated key-admin trust factory is not invented here.
fn expected_scope_on(
    conn: &Connection,
    room: &str,
    recipient: &str,
    publisher: &str,
    generation: u64,
    transport: u64,
    now: u64,
) -> Result<(Vec<u8>, Member, Member), StoreError> {
    let invalid = || StoreError::SeatedAuthorization;
    let (auth, key, current_transport, wallet): (i64,i64,i64,Vec<u8>) = conn.query_row(
        "SELECT auth_generation,key_generation,transport_generation,owner_wallet FROM seated_rooms WHERE room_id=?1",
        [room], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional()?.ok_or_else(invalid)?;
    if auth < 0
        || key < 0
        || current_transport < 0
        || generation > key as u64
        || transport != current_transport as u64
    {
        return Err(invalid());
    }
    let recipient_member = member_on(conn, room, recipient, "read", now)?;
    let publisher_member = member_on(conn, room, publisher, "manage", now)?;
    let r = &recipient_member.context;
    let p = &publisher_member.context;
    let owner_seat = format!(
        "0x{}",
        wallet
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    if wallet.len() != 20
        || p.role != "owner"
        || p.seat != owner_seat
        || p.chain_id != r.chain_id
        || !matches!(r.role.as_str(), "owner" | "builder")
        || generation < recipient_member.from_gen as u64
    {
        return Err(invalid());
    }
    let id = certificates::certificate_id(certificates::IDENTITY, &recipient_member.identity)
        .map_err(|_| invalid())?;
    if id.iter().map(|b| format!("{b:02x}")).collect::<String>() != recipient {
        return Err(invalid());
    }
    let recipient_key =
        certificates::recipient_key(&recipient_member.identity, &id).map_err(|_| invalid())?;
    let text = |s: &str| Value::Text(s.into());
    let value = Value::Map(vec![
        (text("v"), 1u64.into()),
        (text("chain_id"), r.chain_id.into()),
        (text("room"), text(room)),
        (text("auth_generation"), (auth as u64).into()),
        (text("key_generation"), generation.into()),
        (text("transport_generation"), transport.into()),
        (text("recipient_seat"), text(&r.seat)),
        (text("recipient_cert"), text(recipient)),
        (text("recipient_key"), Value::Bytes(recipient_key)),
        (text("publisher_cert"), text(publisher)),
        (text("purpose"), text("room-generation-secret")),
    ]);
    let mut encoded = Vec::new();
    ciborium::into_writer(&value, &mut encoded).map_err(|_| invalid())?;
    let scope = canonical::canonicalize(&encoded).map_err(|_| invalid())?;
    Ok((scope, recipient_member, publisher_member))
}

impl Store {
    pub(crate) fn seated_key_envelope(
        &self,
        room: &str,
        recipient: &str,
        generation: &str,
        caller: &str,
        method: &str,
        target: &str,
        body: &[u8],
        projection: &[u8],
        signature: &[u8],
        now_ms: i64,
    ) -> Result<serde_json::Value, StoreError> {
        let invalid = || StoreError::SeatedAuthorization;
        if now_ms < 0 || body.len() > 16 * 1024 {
            return Err(invalid());
        }
        let key_gen: u64 = generation.parse().map_err(|_| invalid())?;
        if key_gen > i64::MAX as u64 || key_gen.to_string() != generation {
            return Err(invalid());
        }
        let base = format!("/rooms/{room}/key-envelopes/{recipient}/{generation}");
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current_auth: i64 = tx
            .query_row(
                "SELECT auth_generation FROM seated_rooms WHERE room_id=?1",
                [room],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(invalid)?;
        let (input, transport) = if method == "PUT" && target == base {
            let input: crate::seated::KeyEnvelope =
                serde_json::from_slice(body).map_err(|_| invalid())?;
            let transport = input.transport_generation;
            (input, transport)
        } else if method == "GET" && body.is_empty() && caller == recipient {
            let prefix = format!("{base}?transport_generation=");
            let raw = target.strip_prefix(&prefix).ok_or_else(invalid)?;
            let transport: u64 = raw.parse().map_err(|_| invalid())?;
            if transport.to_string() != raw {
                return Err(invalid());
            }
            // Authenticate before looking up existence or returning wrapped bytes.
            let reader = member_on(&tx, room, caller, "read", now_ms as u64)?;
            let token = request::verify_received(
                projection,
                &reader.public,
                signature,
                now_ms as u64,
                method,
                target,
                body,
            )
            .map_err(|_| invalid())?;
            claim_nonce_on(&tx, &token, &reader.public, now_ms)?;
            let raw: Option<(Vec<u8>,Vec<u8>,Vec<u8>)> = tx.query_row(
                "SELECT scope,wrapped,signature FROM seated_key_envelopes WHERE room_id=?1 AND recipient_cert=?2 AND key_generation=?3 AND auth_generation=?4 AND transport_generation=?5",
                params![room,recipient,key_gen as i64,current_auth,transport], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            ).optional()?;
            let (scope, wrapped, sig) = raw.ok_or_else(invalid)?;
            let fields: std::collections::BTreeMap<String, Value> =
                ciborium::from_reader(scope.as_slice()).map_err(|_| invalid())?;
            let Some(Value::Text(publisher)) = fields.get("publisher_cert") else {
                return Err(invalid());
            };
            (
                crate::seated::KeyEnvelope {
                    publisher_cert: publisher.clone(),
                    transport_generation: transport,
                    scope: B64.encode(scope),
                    wrapped: B64.encode(wrapped),
                    signature: B64.encode(sig),
                },
                transport,
            )
        } else {
            return Err(invalid());
        };
        let scope = B64.decode(&input.scope).map_err(|_| invalid())?;
        let wrapped = B64.decode(&input.wrapped).map_err(|_| invalid())?;
        let envelope_signature = B64.decode(&input.signature).map_err(|_| invalid())?;
        let (expected, _, publisher) = expected_scope_on(
            &tx,
            room,
            recipient,
            &input.publisher_cert,
            key_gen,
            transport,
            now_ms as u64,
        )?;
        if scope != expected {
            return Err(invalid());
        }
        key_envelope::verify(&scope, &wrapped, &envelope_signature, &publisher.public)
            .map_err(|_| invalid())?;
        if method == "PUT" {
            if caller != input.publisher_cert {
                return Err(invalid());
            }
            let token = request::verify_received(
                projection,
                &publisher.public,
                signature,
                now_ms as u64,
                method,
                target,
                body,
            )
            .map_err(|_| invalid())?;
            claim_nonce_on(&tx, &token, &publisher.public, now_ms)?;
            let original: Option<(Vec<u8>,Vec<u8>,Vec<u8>)> = tx.query_row(
                "SELECT scope,wrapped,signature FROM seated_key_envelopes WHERE room_id=?1 AND recipient_cert=?2 AND key_generation=?3 AND auth_generation=?4 AND transport_generation=?5",
                params![room,recipient,key_gen as i64,current_auth,transport], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            ).optional()?;
            if let Some(original) = original {
                if original != (scope, wrapped, envelope_signature) {
                    return Err(StoreError::MessageConflict);
                }
            } else {
                tx.execute("INSERT INTO seated_key_envelopes(room_id,recipient_cert,key_generation,auth_generation,transport_generation,scope,wrapped,signature) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    params![room,recipient,key_gen as i64,current_auth,transport,scope,wrapped,envelope_signature])?;
            }
        }
        tx.commit()?;
        Ok(serde_json::to_value(input)?)
    }
}
