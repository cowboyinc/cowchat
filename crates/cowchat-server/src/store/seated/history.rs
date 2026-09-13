use super::*;

impl Store {
    pub(crate) fn read_seated_history(
        &self,
        room: &str,
        cert: &str,
        method: &str,
        raw_target: &str,
        projection: &[u8],
        signature: &[u8],
        query: &crate::seated::HistoryQuery,
        now_ms: i64,
    ) -> Result<serde_json::Value, StoreError> {
        let invalid = || StoreError::SeatedAuthorization;
        if now_ms < 0
            || method != "GET"
            || raw_target.split('?').next() != Some(format!("/rooms/{room}/messages").as_str())
            || query.after < 0
            || query.limit == 0
            || query.limit > 100
        {
            return Err(invalid());
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (public,context,generation):(Vec<u8>,Vec<u8>,i64)=tx.query_row(
            "SELECT c.public_key,c.trusted_context,r.transport_generation FROM seated_credentials c JOIN seated_rooms r ON r.room_id=c.room_id
             WHERE c.room_id=?1 AND c.cert_id=?2 AND c.auth_generation=r.auth_generation",
            params![room,cert],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional()?.ok_or_else(invalid)?;
        if generation < 0 || query.transport_generation != generation as u64 {
            return Err(invalid());
        }
        authorization::verify_member_access(&context, "read", now_ms as u64)
            .map_err(|_| invalid())?;
        let token = request::verify_received(
            projection,
            &public,
            signature,
            now_ms as u64,
            method,
            raw_target,
            b"",
        )
        .map_err(|_| invalid())?;
        claim_nonce_on(&tx, &token, &public, now_ms)?;
        let mut records = Vec::new();
        let mut position = query.after;
        let mut response_bytes = 0;
        {
            let mut statement=tx.prepare("SELECT message_id,room_id,agent_id,agent_name,content,reply_to_message,metadata,created_at,seq
                FROM messages WHERE room_id=?1 AND seq>?2 ORDER BY seq ASC LIMIT ?3")?;
            let rows =
                statement.query_map(params![room, query.after, query.limit], map_message_row)?;
            for message in rows {
                let message = message?;
                let header = message
                    .metadata
                    .get("header_cbor")
                    .and_then(|value| value.as_str())
                    .ok_or_else(invalid)?;
                let bytes = B64.decode(header).map_err(|_| invalid())?;
                let header: crate::seated::RecordHeader =
                    ciborium::from_reader(bytes.as_slice()).map_err(|_| invalid())?;
                let mut record = serde_json::to_value(header)?;
                record["body"] = message.content.into();
                record["sig"] = message.metadata.get("sig").cloned().ok_or_else(invalid)?;
                let framed = serde_json::json!({"record":record,"position":message.seq,"timestamp":message.timestamp});
                let size = serde_json::to_vec(&framed)?.len();
                if response_bytes + size > 4 * 1024 * 1024 {
                    break;
                }
                response_bytes += size;
                position = message.seq;
                records.push(framed);
            }
        }
        tx.commit()?;
        Ok(
            serde_json::json!({"cursor":{"room":room,"transport_generation":generation,"position":position},"records":records}),
        )
    }
}
