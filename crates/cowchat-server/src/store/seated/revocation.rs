use super::*;
use crate::actor_proof::VerifiedActorAbsence;

impl Store {
    /// Proof-authenticated deletion of the control key revokes all actor seats.
    /// A later Present requires a strictly newer authorization generation.
    pub fn ingest_actor_absence(
        &self,
        proof: &VerifiedActorAbsence,
        now: i64,
    ) -> Result<usize, StoreError> {
        let invalid = || StoreError::SeatedAuthorization;
        if now < 0 || !proof.is_fresh_at(now as u64) {
            return Err(invalid());
        }
        let chain = i64::try_from(proof.chain_id()).map_err(|_| invalid())?;
        let height = i64::try_from(proof.height()).map_err(|_| invalid())?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (instance, floor, hash, root): (Vec<u8>,i64,Vec<u8>,Vec<u8>)=tx.query_row(
            "SELECT chain_instance,height,block_hash,state_root FROM seated_actor_floors WHERE chain_id=?1 AND actor=?2",
            params![chain,proof.actor().as_slice()], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).optional()?.ok_or_else(invalid)?;
        if instance != proof.chain_instance()
            || height < floor
            || (height == floor && (hash != proof.block_hash() || root != proof.state_root()))
        {
            return Err(invalid());
        }
        let seat = format!(
            "0x{}",
            proof
                .actor()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        tx.execute(
            "UPDATE subscriptions SET status='failed',revision=revision+1
            WHERE subscription_id IN (SELECT s.subscription_id FROM seated_subscriptions s
            JOIN seated_credentials c ON c.room_id=s.room_id AND c.cert_id=s.cert_id
            WHERE c.actor_chain_id=?1 AND c.seat=?2)",
            params![chain, seat],
        )?;
        let removed = tx.execute(
            "DELETE FROM seated_credentials WHERE actor_chain_id=?1 AND seat=?2",
            params![chain, seat],
        )?;
        tx.execute("UPDATE seated_actor_floors SET height=?3,block_hash=?4,state_root=?5,proof_timestamp=?6,absent=1
            WHERE chain_id=?1 AND actor=?2",params![chain,proof.actor().as_slice(),height,proof.block_hash().as_slice(),proof.state_root().as_slice(),i64::try_from(proof.timestamp()).map_err(|_|invalid())?])?;
        tx.commit()?;
        Ok(removed)
    }

    /// Page actors with credentials to refresh; no scan of message history.
    /// Tuple ordering prevents a permanently failing early actor starving others.
    pub(crate) fn actors_for_refresh(
        &self,
        after: Option<(i64, [u8; 20])>,
        limit: i64,
    ) -> Result<Vec<(i64, [u8; 20])>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let (chain, actor) = after.unwrap_or((-1, [0; 20]));
        let mut stmt=conn.prepare("SELECT f.chain_id,f.actor FROM seated_actor_floors f
            WHERE (f.chain_id>?1 OR (f.chain_id=?1 AND f.actor>?2))
            AND EXISTS(SELECT 1 FROM seated_credentials c WHERE c.actor_chain_id=f.chain_id AND c.seat='0x'||lower(hex(f.actor)))
            ORDER BY f.chain_id,f.actor LIMIT ?3")?;
        let rows = stmt.query_map(params![chain, actor.as_slice(), limit], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;
        rows.map(|row| {
            let (chain, actor) = row?;
            Ok((
                chain,
                actor
                    .try_into()
                    .map_err(|_| StoreError::SeatedAuthorization)?,
            ))
        })
        .collect()
    }
}

/// Authority-changing subscription operations consult only this cached proof.
/// Steady-state reads/appends/wakes retain the last successfully verified state.
pub(super) fn require_recent_actor_control_on(
    conn: &Connection,
    room: &str,
    cert: &str,
    now: i64,
) -> Result<(), StoreError> {
    let invalid = || StoreError::SeatedAuthorization;
    let row: Option<(Option<i64>,String,i64,Vec<u8>)>=conn.query_row(
        "SELECT actor_chain_id,seat,COALESCE(actor_authorization_generation,-1),actor_controller
        FROM seated_credentials WHERE room_id=?1 AND cert_id=?2",params![room,cert],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional()?;
    let Some((chain, seat, generation, controller)) = row else {
        return Err(invalid());
    };
    let Some(chain) = chain else {
        return Ok(());
    };
    let stamp:Option<i64>=conn.query_row("SELECT proof_timestamp FROM seated_actor_floors
        WHERE chain_id=?1 AND '0x'||lower(hex(actor))=?2 AND authorization_generation=?3 AND controller=?4 AND absent=0",
        params![chain,seat,generation,controller],|r|r.get(0),
    ).optional()?;
    let stamp = stamp.ok_or_else(invalid)?;
    if now < 0
        || stamp <= 0
        || stamp > now.saturating_add(5_000)
        || now.saturating_sub(stamp) > 60_000
    {
        return Err(invalid());
    }
    Ok(())
}
