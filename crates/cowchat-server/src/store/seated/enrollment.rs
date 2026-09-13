use super::*;
use cowchat_crypto::certificates;

impl Store {
    pub(crate) fn enroll_seated_owner(
        &self,
        room: &str,
        api_key: &str,
        raw: &[u8],
        now_ms: i64,
    ) -> Result<(String, String), StoreError> {
        let invalid = || StoreError::SeatedAuthorization;
        if raw.len() > 512 * 1024
            || now_ms < 0
            || api_key.is_empty()
            || uuid::Uuid::parse_str(room).is_err()
        {
            return Err(invalid());
        }
        let input: crate::seated::OwnerEnrollment =
            serde_json::from_slice(raw).map_err(|_| invalid())?;
        let identity = B64.decode(input.identity).map_err(|_| invalid())?;
        let identity_signature = B64
            .decode(input.identity_signature)
            .map_err(|_| invalid())?;
        let membership = B64.decode(input.membership).map_err(|_| invalid())?;
        let membership_signature = B64
            .decode(input.membership_signature)
            .map_err(|_| invalid())?;
        let verified = certificates::verify_initial_owner(
            room,
            &identity,
            &identity_signature,
            &membership,
            &membership_signature,
            now_ms as u64,
        )
        .map_err(|_| invalid())?;
        let (id, seat, public, context, wallet): (Vec<u8>, String, Vec<u8>, Vec<u8>, Vec<u8>) =
            ciborium::from_reader(verified.as_slice()).map_err(|_| invalid())?;
        let cert: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
        // Certificates, rather than JSON whitespace, define the enrollment operation.
        let digest = Sha256::digest(serde_json::to_vec(&(
            &identity,
            &identity_signature,
            &membership,
            &membership_signature,
        ))?)
        .to_vec();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let owned: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM rooms WHERE room_id = ?1 AND owner_key = ?2 AND visibility = 'private')", params![room, api_key], |row| row.get(0))?;
        if !owned {
            return Err(invalid());
        }
        let existing: Option<(Vec<u8>, i64, i64)> = tx.query_row(
            "SELECT enrollment_digest, auth_generation, key_generation FROM seated_rooms WHERE room_id = ?1", [room],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        if let Some((original, auth, key)) = existing {
            if original == digest && auth == 0 && key == 0 {
                return Ok((seat, cert));
            }
            return Err(StoreError::MessageConflict);
        }
        let has_content: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM room_sequences WHERE room_id = ?1 AND high_water > 0)
             OR EXISTS(SELECT 1 FROM subscriptions WHERE room_id = ?1)
             OR EXISTS(SELECT 1 FROM blobs WHERE room_id = ?1)",
            [room],
            |row| row.get(0),
        )?;
        if has_content {
            return Err(StoreError::MessageConflict);
        }
        tx.execute("INSERT INTO seated_rooms(room_id, auth_generation, key_generation, owner_wallet, enrollment_digest) VALUES (?1, 0, 0, ?2, ?3)", params![room, wallet, digest])?;
        tx.execute("INSERT INTO seated_credentials(room_id, cert_id, seat, display_name, auth_generation, public_key, trusted_context,
                identity_certificate, identity_signature, membership_certificate, membership_signature)
            VALUES (?1, ?2, ?3, 'Owner', 0, ?4, ?5, ?6, ?7, ?8, ?9)", params![room, cert, seat, public, context, identity, identity_signature, membership, membership_signature])?;
        tx.execute("UPDATE rooms SET encrypted = 1 WHERE room_id = ?1", [room])?;
        tx.commit()?;
        Ok((seat, cert))
    }
}
