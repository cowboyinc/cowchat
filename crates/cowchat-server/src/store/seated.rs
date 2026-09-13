//! Durable signed-append boundary. All state here is local service state.
//! No wire endpoint may insert a trusted credential context directly.
use super::*;
use base64::{engine::general_purpose::STANDARD_NO_PAD as B64, Engine};
use cowchat_crypto::{authorization, request};

mod enrollment;
mod history;
mod subscriptions;
pub(super) use subscriptions::record_wake_on;

pub(super) fn initialize(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS seated_rooms (
        room_id TEXT PRIMARY KEY REFERENCES rooms(room_id) ON DELETE CASCADE,
        auth_generation INTEGER NOT NULL, key_generation INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS seated_credentials (
        room_id TEXT NOT NULL REFERENCES seated_rooms(room_id) ON DELETE CASCADE,
        cert_id TEXT NOT NULL, seat TEXT NOT NULL, display_name TEXT NOT NULL,
        auth_generation INTEGER NOT NULL, public_key BLOB NOT NULL,
        trusted_context BLOB NOT NULL, PRIMARY KEY(room_id, cert_id)
    );
    CREATE TABLE IF NOT EXISTS seated_request_nonces (
        public_key BLOB NOT NULL, nonce BLOB NOT NULL, retain_through_ms INTEGER NOT NULL,
        PRIMARY KEY(public_key, nonce)
    );
    CREATE TABLE IF NOT EXISTS seated_subscriptions (
        subscription_id TEXT PRIMARY KEY REFERENCES subscriptions(subscription_id) ON DELETE CASCADE,
        room_id TEXT NOT NULL REFERENCES seated_rooms(room_id) ON DELETE CASCADE,
        cert_id TEXT NOT NULL, seat TEXT NOT NULL, auth_generation INTEGER NOT NULL,
        transport_generation INTEGER NOT NULL, request_digest BLOB NOT NULL,
        UNIQUE(room_id, seat)
    );
    CREATE TABLE IF NOT EXISTS seated_wakes (
        delivery_id TEXT PRIMARY KEY REFERENCES subscription_deliveries(delivery_id) ON DELETE CASCADE,
        payload TEXT NOT NULL
    );",
    )?;
    ensure_column_exists(
        conn,
        "seated_rooms",
        "owner_wallet",
        "BLOB NOT NULL DEFAULT X''",
    )?;
    ensure_column_exists(
        conn,
        "seated_rooms",
        "enrollment_digest",
        "BLOB NOT NULL DEFAULT X''",
    )?;
    ensure_column_exists(
        conn,
        "seated_rooms",
        "transport_generation",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    for column in [
        "identity_certificate",
        "identity_signature",
        "membership_certificate",
        "membership_signature",
    ] {
        ensure_column_exists(
            conn,
            "seated_credentials",
            column,
            "BLOB NOT NULL DEFAULT X''",
        )?;
    }
    Ok(())
}

pub(super) fn is_seated_on(conn: &Connection, room: &str) -> Result<bool, StoreError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM seated_rooms WHERE room_id = ?1)",
        [room],
        |row| row.get(0),
    )?)
}

impl Store {
    pub(crate) fn is_seated_room(&self, room: &str) -> Result<bool, StoreError> {
        is_seated_on(&self.conn.lock().unwrap(), room)
    }

    pub(crate) fn append_seated_record(
        &self,
        room: &str,
        method: &str,
        raw_target: &str,
        body: &[u8],
        projection: &[u8],
        signature: &[u8],
        now_ms: i64,
    ) -> Result<AppendResult, StoreError> {
        let invalid = || StoreError::SeatedAuthorization;
        if now_ms < 0
            || method != "POST"
            || raw_target.split('?').next() != Some(format!("/rooms/{room}/messages").as_str())
        {
            return Err(invalid());
        }
        let record = crate::seated::SealedRecord::parse(body)?;
        if record.header.room != room {
            return Err(invalid());
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row = tx
            .query_row(
                "SELECT c.seat, c.display_name, c.public_key, c.trusted_context, r.key_generation
             FROM seated_credentials c JOIN seated_rooms r ON r.room_id = c.room_id
             WHERE c.room_id = ?1 AND c.cert_id = ?2 AND c.auth_generation = r.auth_generation",
                params![room, record.header.cert],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(invalid)?;
        let (principal, mut display_name, public_key, context, generation) = row;
        if principal != record.header.seat {
            // A forwarding door signs as itself but displays the verified owner
            // it speaks for. Never substitute the door's name for that owner.
            display_name = tx.query_row(
                "SELECT c.display_name FROM seated_credentials c JOIN seated_rooms r ON r.room_id = c.room_id
                 WHERE c.room_id = ?1 AND c.seat = ?2 AND c.auth_generation = r.auth_generation LIMIT 1",
                params![room, record.header.seat], |row| row.get(0),
            ).optional()?.ok_or_else(invalid)?;
        }
        if generation < 0 || record.header.gen != generation as u64 {
            return Err(invalid());
        }
        let token = request::verify_received(
            projection,
            &public_key,
            signature,
            now_ms as u64,
            method,
            raw_target,
            body,
        )
        .map_err(|_| invalid())?;
        authorization::verify_member_record(
            &record.header_cbor,
            &record.body,
            &record.signature,
            &context,
            now_ms as u64,
        )
        .map_err(|_| invalid())?;
        // Recheck authority and consume the request nonce in the same transaction
        // as the append/outbox; no revocation or crash can split these effects.
        claim_nonce_on(&tx, &token, &public_key, now_ms)?;
        let metadata = serde_json::json!({"v":3, "header_cbor":B64.encode(&record.header_cbor),
            "sig":B64.encode(&record.signature), "type":record.header.class,
            "wake_hint":record.header.wake_hint, "signer_seat":principal});
        let append = MessageAppend {
            message_id: &record.header.message_id,
            room_id: room,
            agent_id: &record.header.seat,
            agent_name: &display_name,
            content: &record.body,
            reply_to: record.header.reply_to.as_deref(),
            metadata: &metadata,
            mentions: &record.header.mentions,
        };
        let result = Self::append_on(&tx, &append)?;
        tx.commit()?;
        Ok(result)
    }
}

fn claim_nonce_on(
    tx: &Connection,
    token: &[u8],
    public_key: &[u8],
    now_ms: i64,
) -> Result<(), StoreError> {
    let invalid = || StoreError::SeatedAuthorization;
    let token: (Vec<u8>, Vec<u8>, u64) = ciborium::from_reader(token).map_err(|_| invalid())?;
    if token.0.as_slice() != public_key {
        return Err(invalid());
    }
    let retain = i64::try_from(token.2).map_err(|_| invalid())?;
    tx.execute(
        "DELETE FROM seated_request_nonces WHERE retain_through_ms < ?1",
        [now_ms],
    )?;
    if tx.execute(
        "INSERT INTO seated_request_nonces(public_key, nonce, retain_through_ms)
        VALUES (?1, ?2, ?3) ON CONFLICT(public_key, nonce) DO NOTHING",
        params![public_key, token.1, retain],
    )? == 0
    {
        return Err(StoreError::SeatedReplay);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
