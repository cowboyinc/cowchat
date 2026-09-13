//! Authenticated, mentions-only subscriptions. These deliver notifications; they
//! do not authorize paid compute or any other external effect.
use super::*;
use sha2::{Digest, Sha256};

impl Store {
    // Check before asynchronous URL validation, so anonymous callers cannot
    // trigger DNS work. The write transaction independently rechecks authority.
    pub(crate) fn preflight_seated_subscription(
        &self,
        room: &str,
        cert: &str,
        method: &str,
        target: &str,
        body: &[u8],
        projection: &[u8],
        signature: &[u8],
        now: i64,
    ) -> Result<(), StoreError> {
        let invalid = || StoreError::SeatedAuthorization;
        if now < 0 || method != "POST" || target != format!("/rooms/{room}/subscriptions") {
            return Err(invalid());
        }
        let conn = self.conn.lock().unwrap();
        let (public,context):(Vec<u8>,Vec<u8>)=conn.query_row(
            "SELECT c.public_key,c.trusted_context FROM seated_credentials c JOIN seated_rooms r ON r.room_id=c.room_id
             WHERE c.room_id=?1 AND c.cert_id=?2 AND c.auth_generation=r.auth_generation",
            params![room,cert],|row| Ok((row.get(0)?,row.get(1)?)),
        ).optional()?.ok_or_else(invalid)?;
        authorization::verify_member_access(&context, "read", now as u64).map_err(|_| invalid())?;
        request::verify_received(
            projection, &public, signature, now as u64, method, target, body,
        )
        .map_err(|_| invalid())?;
        Ok(())
    }
    pub(crate) fn subscribe_seated(
        &self,
        room: &str,
        cert: &str,
        method: &str,
        target: &str,
        body: &[u8],
        projection: &[u8],
        signature: &[u8],
        now: i64,
    ) -> Result<serde_json::Value, StoreError> {
        let invalid = || StoreError::SeatedAuthorization;
        if now < 0 || method != "POST" || target != format!("/rooms/{room}/subscriptions") {
            return Err(invalid());
        }
        let input: crate::seated::MentionSubscription =
            serde_json::from_slice(body).map_err(|_| invalid())?;
        input.validate().map_err(|_| invalid())?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (seat,public,context,auth,transport):(String,Vec<u8>,Vec<u8>,i64,i64)=tx.query_row(
            "SELECT c.seat,c.public_key,c.trusted_context,r.auth_generation,r.transport_generation
             FROM seated_credentials c JOIN seated_rooms r ON r.room_id=c.room_id
             WHERE c.room_id=?1 AND c.cert_id=?2 AND c.auth_generation=r.auth_generation",
            params![room,cert],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        ).optional()?.ok_or_else(invalid)?;
        if transport < 0 || input.transport_generation != transport as u64 {
            return Err(invalid());
        }
        authorization::verify_member_access(&context, "read", now as u64).map_err(|_| invalid())?;
        let token = request::verify_received(
            projection, &public, signature, now as u64, method, target, body,
        )
        .map_err(|_| invalid())?;
        claim_nonce_on(&tx, &token, &public, now)?;
        let digest = Sha256::digest(body).to_vec();
        let existing:Option<(String,String,Vec<u8>,i64,i64)>=tx.query_row(
            "SELECT room_id,cert_id,request_digest,auth_generation,transport_generation FROM seated_subscriptions WHERE subscription_id=?1",
            [&input.subscription_id],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        ).optional()?;
        if let Some((old_room, old_cert, old_digest, old_auth, old_transport)) = existing {
            if old_auth != auth || old_transport != transport {
                return Err(invalid());
            }
            if old_room != room || old_cert != cert || old_digest != digest {
                return Err(StoreError::MessageConflict);
            }
            // Retries acknowledge the existing subscription without reviving it,
            // changing its endpoint, or resetting its cursor.
        } else {
            let collision: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM subscriptions WHERE subscription_id=?1)
                 OR EXISTS(SELECT 1 FROM seated_subscriptions WHERE room_id=?2 AND seat=?3)",
                params![input.subscription_id, room, seat],
                |row| row.get(0),
            )?;
            if collision {
                return Err(StoreError::MessageConflict);
            }
            let tip: i64 = tx.query_row(
                "SELECT COALESCE((SELECT high_water FROM room_sequences WHERE room_id=?1),0)",
                [room],
                |row| row.get(0),
            )?;
            let after = input.after.unwrap_or(tip);
            if after > tip {
                return Err(invalid());
            }
            tx.execute(
                "INSERT INTO subscriptions(subscription_id,room_id,owner_key,webhook_url,secret,
                 exclude_thinking,since_seq,last_delivered_seq,status,failure_count,created_at,only_mention)
                 VALUES (?1,?2,'',?3,?4,1,?5,?5,'active',0,?6,?7)",
                params![input.subscription_id,room,input.webhook_url,input.secret,after,Utc::now().to_rfc3339(),seat],
            )?;
            tx.execute(
                "INSERT INTO seated_subscriptions VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    input.subscription_id,
                    room,
                    cert,
                    seat,
                    auth,
                    transport,
                    digest
                ],
            )?;
            let sub=tx.query_row(
                "SELECT subscription_id,room_id,owner_key,webhook_url,secret,kinds,only_from,not_from,
                 exclude_thinking,since_seq,last_delivered_seq,status,failure_count,created_at,only_mention
                 FROM subscriptions WHERE subscription_id=?1",[&input.subscription_id],map_subscription_row,
            )?.0;
            enqueue_subscription_backfill_on(&tx, &sub)?;
        }
        let (position, status): (i64, String) = tx.query_row(
            "SELECT last_delivered_seq,status FROM subscriptions WHERE subscription_id=?1",
            [&input.subscription_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        tx.commit()?;
        Ok(
            serde_json::json!({"subscription_id":input.subscription_id,"seat":seat,"status":status,
            "cursor":{"room":room,"transport_generation":transport,"position":position}}),
        )
    }

    /// Recheck current membership immediately before dispatch. A revocation can
    /// race an already-sent notification; its subsequent signed read still fails.
    pub(crate) fn seated_wake_payload(
        &self,
        subscription: &str,
        delivery: &str,
        now: i64,
    ) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let room: String = conn.query_row(
            "SELECT room_id FROM subscriptions WHERE subscription_id=?1",
            [subscription],
            |row| row.get(0),
        )?;
        if !is_seated_on(&conn, &room)? {
            return Ok(None);
        }
        let context:Vec<u8>=conn.query_row(
            "SELECT c.trusted_context FROM seated_subscriptions s
             JOIN seated_rooms r ON r.room_id=s.room_id
             JOIN seated_credentials c ON c.room_id=s.room_id AND c.cert_id=s.cert_id AND c.seat=s.seat
             WHERE s.subscription_id=?1 AND s.auth_generation=r.auth_generation
             AND c.auth_generation=r.auth_generation AND s.transport_generation=r.transport_generation",
            [subscription],|row| row.get(0),
        ).optional()?.ok_or(StoreError::SeatedAuthorization)?;
        if now < 0 {
            return Err(StoreError::SeatedAuthorization);
        }
        authorization::verify_member_access(&context, "read", now as u64)
            .map_err(|_| StoreError::SeatedAuthorization)?;
        let payload=conn.query_row(
            "SELECT w.payload FROM seated_wakes w JOIN subscription_deliveries d ON d.delivery_id=w.delivery_id
             WHERE w.delivery_id=?1 AND d.subscription_id=?2",params![delivery,subscription],|row| row.get(0),
        )?;
        Ok(Some(payload))
    }
}

/// Called in the same transaction as the obligation. Persist the exact bytes so
/// later cursor movement or restart cannot change a retry's authenticated body.
pub(in crate::store) fn record_wake_on(
    conn: &Connection,
    delivery: &str,
    subscription: &str,
    message: &str,
    seq: i64,
) -> Result<(), StoreError> {
    let bound:Option<(String,i64,i64)>=conn.query_row(
        "SELECT s.room_id,s.transport_generation,b.last_delivered_seq FROM seated_subscriptions s
         JOIN subscriptions b ON b.subscription_id=s.subscription_id WHERE s.subscription_id=?1",
        [subscription],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional()?;
    if let Some((room, generation, since)) = bound {
        let payload = serde_json::json!({"specversion":"1.0","id":delivery,"source":format!("/rooms/{room}"),
            "type":"cowchat.room.wake","datacontenttype":"application/json",
            "data":{"room":room,"message_id":message,"seq":seq,"tip":seq,"since_seq":since,
            "dispatch_id":delivery,"transport_generation":generation}});
        conn.execute(
            "INSERT INTO seated_wakes(delivery_id,payload) VALUES (?1,?2)",
            params![delivery, serde_json::to_string(&payload)?],
        )?;
    }
    Ok(())
}
