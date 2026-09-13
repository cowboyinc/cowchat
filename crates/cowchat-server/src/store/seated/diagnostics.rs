//! A local metadata snapshot, not a transport/archive/runtime health attestation.
use super::*;

impl Store {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn read_seated_diagnostics(
        &self,
        room: &str,
        cert: &str,
        method: &str,
        target: &str,
        projection: &[u8],
        signature: &[u8],
        transport_generation: u64,
        now: i64,
    ) -> Result<serde_json::Value, StoreError> {
        let invalid = || StoreError::SeatedAuthorization;
        if now < 0
            || method != "GET"
            || target.split('?').next() != Some(format!("/rooms/{room}/diagnostics").as_str())
        {
            return Err(invalid());
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (seat, public, context, auth, key, transport): (String, Vec<u8>, Vec<u8>, i64, i64, i64) = tx.query_row(
            "SELECT c.seat,c.public_key,c.trusted_context,r.auth_generation,r.key_generation,r.transport_generation
             FROM seated_credentials c JOIN seated_rooms r ON r.room_id=c.room_id
             WHERE c.room_id=?1 AND c.cert_id=?2 AND c.auth_generation=r.auth_generation",
            params![room, cert], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        ).optional()?.ok_or_else(invalid)?;
        if auth < 0 || key < 0 || transport < 0 || transport_generation != transport as u64 {
            return Err(invalid());
        }
        authorization::verify_member_access(&context, "read", now as u64).map_err(|_| invalid())?;
        let token = request::verify_received(
            projection, &public, signature, now as u64, method, target, b"",
        )
        .map_err(|_| invalid())?;
        claim_nonce_on(&tx, &token, &public, now)?;
        let tip: i64 = tx.query_row(
            "SELECT COALESCE((SELECT high_water FROM room_sequences WHERE room_id=?1),0)",
            [room],
            |r| r.get(0),
        )?;
        // Select only allowlisted metadata. Never hydrate callback URLs, secrets,
        // message content, wake payloads or persisted error strings for this read.
        let subscription: Option<(String, bool, i64)> = tx.query_row(
            "SELECT b.status,(s.cert_id=?3 AND s.auth_generation=?4 AND s.transport_generation=?5),
               (SELECT COUNT(*) FROM subscription_deliveries d JOIN seated_wakes w ON w.delivery_id=d.delivery_id
                WHERE d.subscription_id=s.subscription_id AND d.status='pending')
             FROM seated_subscriptions s JOIN subscriptions b ON b.subscription_id=s.subscription_id
             WHERE s.room_id=?1 AND s.seat=?2 AND b.room_id=s.room_id",
            params![room, seat, cert, auth, transport], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional()?;
        let subscription = subscription.map(|(status, binding_current, pending)| {
            let state = match status.as_str() {
                "active" => "active",
                "failed" => "failed",
                "disabled" => "disabled",
                _ => "unknown",
            };
            serde_json::json!({"state":state,"binding_current":binding_current,"pending_wakes":pending})
        });
        tx.commit()?;
        Ok(serde_json::json!({
            "version":1,
            "transport":{"kind":"sqlite","generation":transport,"local_tip":tip},
            "authorization_generation":auth,"key_generation":key,
            "subscription":subscription,
            "checks":{"archive":"unsupported","production_runtime":"unsupported"}
        }))
    }
}
