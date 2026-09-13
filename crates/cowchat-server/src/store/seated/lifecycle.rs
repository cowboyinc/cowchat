//! Local, signed subscription lifecycle. Operation receipts survive deletion;
//! revisions fence late webhook responses without a chain transaction.
use super::*;
use crate::seated::{SubscriptionAction, SubscriptionMutation};
use sha2::{Digest, Sha256};

impl Store {
    pub(crate) fn mutate_seated_subscription(
        &self,
        room: &str,
        subscription: &str,
        cert: &str,
        method: &str,
        target: &str,
        body: &[u8],
        projection: &[u8],
        signature: &[u8],
        now: i64,
    ) -> Result<serde_json::Value, StoreError> {
        let invalid = || StoreError::SeatedAuthorization;
        if now < 0
            || method != "POST"
            || target != format!("/rooms/{room}/subscriptions/{subscription}/lifecycle")
        {
            return Err(invalid());
        }
        let input: SubscriptionMutation = serde_json::from_slice(body).map_err(|_| invalid())?;
        input.validate().map_err(|_| invalid())?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (seat, public, context, auth, transport): (String, Vec<u8>, Vec<u8>, i64, i64) = tx.query_row(
            "SELECT c.seat,c.public_key,c.trusted_context,r.auth_generation,r.transport_generation
             FROM seated_credentials c JOIN seated_rooms r ON r.room_id=c.room_id
             WHERE c.room_id=?1 AND c.cert_id=?2 AND c.auth_generation=r.auth_generation",
            params![room, cert], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
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
        // Method and target are authenticated above and domain-separate the receipt.
        let digest =
            Sha256::digest([method.as_bytes(), b"\0", target.as_bytes(), b"\0", body].concat())
                .to_vec();
        let previous: Option<(String, String, Vec<u8>, String)> = tx.query_row(
            "SELECT room_id,seat,request_digest,response FROM seated_subscription_operations WHERE operation_id=?1",
            [&input.operation_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).optional()?;
        if let Some((old_room, old_seat, old_digest, response)) = previous {
            if old_room != room || old_seat != seat || old_digest != digest {
                return Err(StoreError::MessageConflict);
            }
            let response = serde_json::from_str(&response)?;
            tx.commit()?;
            return Ok(response);
        }
        let (bound_room, bound_seat, old_transport, revision, position, mut status):
            (String, String, i64, i64, i64, String) = tx.query_row(
            "SELECT s.room_id,s.seat,s.transport_generation,b.revision,b.last_delivered_seq,b.status
             FROM seated_subscriptions s JOIN subscriptions b ON b.subscription_id=s.subscription_id
             WHERE s.subscription_id=?1", [subscription],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        ).optional()?.ok_or_else(invalid)?;
        if bound_room != room || bound_seat != seat {
            return Err(invalid());
        }
        // Transport migration needs a new binding; old immutable wakes name the old transport.
        if (old_transport != transport && !matches!(input.action, SubscriptionAction::Delete))
            || revision != input.expected_revision
        {
            return Err(StoreError::MessageConflict);
        }
        if matches!(input.action, SubscriptionAction::Update { .. }) {
            let current: bool = tx.query_row(
                "SELECT cert_id=?2 AND auth_generation=?3 FROM seated_subscriptions WHERE subscription_id=?1",
                params![subscription,cert,auth], |r| r.get(0),
            )?;
            if !current {
                // A new certificate must repair first so its readable history floor is applied.
                return Err(invalid());
            }
        }
        let next_revision = revision.checked_add(1).ok_or_else(invalid)?;
        let deleted = matches!(input.action, SubscriptionAction::Delete);
        match &input.action {
            SubscriptionAction::Delete => {
                tx.execute(
                    "DELETE FROM subscriptions WHERE subscription_id=?1",
                    [subscription],
                )?;
                status = "deleted".into();
            }
            action => {
                // Renewal is limited to the same seat under current, independently enrolled authority.
                tx.execute("UPDATE seated_subscriptions SET cert_id=?2,auth_generation=?3 WHERE subscription_id=?1",
                    params![subscription,cert,auth])?;
                tx.execute(
                    "UPDATE subscriptions SET revision=?2 WHERE subscription_id=?1",
                    params![subscription, next_revision],
                )?;
                if let SubscriptionAction::Update {
                    webhook_url,
                    secret,
                } = action
                {
                    tx.execute("UPDATE subscriptions SET webhook_url=?2,secret=?3 WHERE subscription_id=?1",
                        params![subscription,webhook_url,secret])?;
                } else {
                    let sub = tx.query_row(
                        "SELECT subscription_id,room_id,owner_key,webhook_url,secret,kinds,only_from,not_from,
                         exclude_thinking,since_seq,last_delivered_seq,status,failure_count,created_at,only_mention
                         FROM subscriptions WHERE subscription_id=?1", [subscription], map_subscription_row,
                    )?.0;
                    enqueue_subscription_backfill_on(&tx, &sub)?;
                    // A new certificate may have a later readable key floor. Do not revive older wakes.
                    let retained = {
                        let mut stmt = tx.prepare(
                            "SELECT delivery_id,message_id FROM subscription_deliveries
                            WHERE subscription_id=?1 AND message_seq>?2",
                        )?;
                        let rows = stmt.query_map(params![subscription, position], |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                        })?;
                        rows.collect::<Result<Vec<_>, _>>()?
                    };
                    for (delivery, message) in retained {
                        if subscriptions::allows_message_on(&tx, subscription, &message)? {
                            tx.execute("UPDATE subscription_deliveries SET status='pending',attempts=0,last_error=NULL,
                                next_attempt_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),
                                deadline_at=strftime('%Y-%m-%dT%H:%M:%fZ','now','+1 day') WHERE delivery_id=?1", [delivery])?;
                        } else {
                            tx.execute("UPDATE subscription_deliveries SET status='abandoned',last_error='outside current readable history' WHERE delivery_id=?1", [delivery])?;
                        }
                    }
                    tx.execute("UPDATE subscriptions SET status='active',failure_count=0 WHERE subscription_id=?1", [subscription])?;
                    status = "active".into();
                }
            }
        }
        let response = serde_json::json!({"subscription_id":subscription,"operation_id":input.operation_id,
            "seat":seat,"status":status,"revision":next_revision,
            "cursor":{"room":room,"transport_generation":transport,"position":position}});
        tx.execute(
            "INSERT INTO seated_subscription_operations VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                input.operation_id,
                room,
                subscription,
                seat,
                digest,
                serde_json::to_string(&response)?,
                deleted
            ],
        )?;
        tx.commit()?;
        Ok(response)
    }
}
