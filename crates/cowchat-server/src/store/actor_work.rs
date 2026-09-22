//! Durable actor processing, separate from webhook delivery acknowledgments.
use super::*;
use cowchat_core::{ActorWork, WakeMode};

// Crash recovery can repeat inference; the stable reply ID prevents double posts.
const CLAIM_SECONDS: i64 = 300;

pub(super) fn initialize(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS actor_subscriptions (
            subscription_id TEXT PRIMARY KEY REFERENCES subscriptions(subscription_id) ON DELETE CASCADE,
            agent_id TEXT NOT NULL,
            mode TEXT NOT NULL CHECK(mode IN ('always', 'addressed', 'listen')),
            processed_seq INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS persistent_rooms (
            room_id TEXT PRIMARY KEY REFERENCES rooms(room_id) ON DELETE CASCADE
        );",
    )?;
    ensure_column_exists(conn, "subscription_deliveries", "claimed_until", "INTEGER")
}

pub(super) fn matches_actor(
    conn: &Connection,
    sub_id: &str,
    append: &MessageAppend<'_>,
) -> Result<bool, StoreError> {
    let actor: Option<(String, String)> = conn
        .query_row(
            "SELECT agent_id, mode FROM actor_subscriptions WHERE subscription_id = ?1",
            [sub_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((agent_id, mode)) = actor else {
        return Ok(true);
    };
    if agent_id == append.agent_id
        || mode == "listen"
        || (mode == "addressed" && !append.mentions.contains(&agent_id))
        || matches!(
            append.metadata.get("type").and_then(|v| v.as_str()),
            Some("thinking" | "system")
        )
    {
        return Ok(false);
    }
    Ok(true)
}

pub(super) fn validate_reply(
    conn: &Connection,
    append: &MessageAppend<'_>,
) -> Result<(), StoreError> {
    let Some(work_id) = append.message_id.strip_prefix("actor-reply:") else {
        return Ok(());
    };
    let valid: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM subscription_deliveries w
         JOIN actor_subscriptions a USING(subscription_id)
         JOIN subscriptions s USING(subscription_id)
         WHERE w.delivery_id = ?1 AND a.agent_id = ?2 AND s.room_id = ?3
           AND s.status = 'active' AND w.status = 'pending' AND w.claimed_until IS NOT NULL
           AND w.message_id = ?4)",
        params![work_id, append.agent_id, append.room_id, append.reply_to],
        |r| r.get(0),
    )?;
    if !valid {
        return Err(StoreError::InvalidActorWork);
    }
    Ok(())
}

impl Store {
    /// Durable participants remain addressable while their runtime is asleep.
    /// This exposes identities only; callers must authorize access to the room.
    pub fn room_actor_participants(
        &self,
        room_id: &str,
    ) -> Result<Vec<(String, String)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT a.agent_id, COALESCE(
                (SELECT m.agent_name FROM messages m WHERE m.room_id = s.room_id
                 AND m.agent_id = a.agent_id ORDER BY m.seq DESC LIMIT 1), a.agent_id)
             FROM actor_subscriptions a JOIN subscriptions s USING(subscription_id)
             WHERE s.room_id = ?1 ORDER BY a.agent_id",
        )?;
        let rows = stmt.query_map([room_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Starts at the current tip: old messages never retroactively cause inference.
    /// The actor identity comes from the authenticated connection, not the payload.
    pub fn create_actor_subscription(
        &self,
        room_id: &str,
        owner_key: &str,
        agent_id: &str,
        webhook_url: &str,
        secret: &str,
        mode: WakeMode,
    ) -> Result<String, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing: Option<(String, String, String, String, String)> = tx.query_row(
            "SELECT s.subscription_id, s.owner_key, s.webhook_url, s.secret, a.mode FROM actor_subscriptions a JOIN subscriptions s USING(subscription_id)
             WHERE s.room_id = ?1 AND a.agent_id = ?2", params![room_id, agent_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        ).optional()?;
        let mode = match mode {
            WakeMode::Always => "always",
            WakeMode::Addressed => "addressed",
            WakeMode::Listen => "listen",
        };
        if let Some((id, owner, url, key, previous_mode)) = existing {
            // Same owner re-enrolling is a restart: the wake endpoint and secret
            // are transport config and may change (new port, moved host). The
            // subscription identity, cursor, and pending work are preserved.
            if owner != owner_key {
                return Err(StoreError::InvalidActorWork);
            }
            if url != webhook_url || key != secret || previous_mode != mode {
                tx.execute(
                    "UPDATE subscriptions SET webhook_url = ?2, secret = ?3, status = 'active', failure_count = 0
                     WHERE subscription_id = ?1",
                    params![id, webhook_url, secret],
                )?;
                tx.execute(
                    "UPDATE actor_subscriptions SET mode = ?2 WHERE subscription_id = ?1",
                    params![id, mode],
                )?;
                tx.commit()?;
            }
            return Ok(id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let tip: i64 = tx.query_row(
            "SELECT COALESCE((SELECT high_water FROM room_sequences WHERE room_id = ?1), 0)",
            [room_id],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO subscriptions(subscription_id, room_id, owner_key, webhook_url, secret,
             exclude_thinking, since_seq, last_delivered_seq, status, failure_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6, 'active', 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![id, room_id, owner_key, webhook_url, secret, tip],
        )?;
        tx.execute("INSERT INTO actor_subscriptions(subscription_id, agent_id, mode, processed_seq) VALUES (?1, ?2, ?3, ?4)", params![id, agent_id, mode, tip])?;
        tx.execute(
            "INSERT OR IGNORE INTO persistent_rooms(room_id) VALUES (?1)",
            [room_id],
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn is_actor_subscription(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM actor_subscriptions WHERE subscription_id = ?1)",
            [id],
            |r| r.get(0),
        )?)
    }

    /// Claim only the oldest unfinished work. A crashed worker's claim expires;
    /// a warm worker and a wake-started worker use this same operation.
    pub fn claim_actor_work(
        &self,
        id: &str,
        agent_id: &str,
        now: i64,
    ) -> Result<Option<ActorWork>, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let work: Option<(String, String, String, i64, Option<i64>)> = tx.query_row(
            "SELECT w.delivery_id, s.room_id, w.message_id, w.message_seq, w.claimed_until
             FROM subscription_deliveries w JOIN actor_subscriptions a USING(subscription_id)
             JOIN subscriptions s USING(subscription_id)
             WHERE w.subscription_id = ?1 AND a.agent_id = ?2 AND s.status = 'active' AND w.status = 'pending'
             ORDER BY w.message_seq LIMIT 1", params![id, agent_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        ).optional()?;
        let Some((work_id, room_id, message_id, message_seq, until)) = work else {
            return Ok(None);
        };
        let message = |id: &str| -> Result<Option<ChatMessage>, StoreError> {
            Ok(tx.query_row("SELECT message_id, room_id, agent_id, agent_name, content, reply_to_message, metadata, created_at, seq FROM messages WHERE message_id = ?1", [id], map_message_row).optional()?)
        };
        let input = message(&message_id)?.ok_or(StoreError::InvalidActorWork)?;
        let existing_reply = message(&format!("actor-reply:{work_id}"))?;
        // A persisted reply needs only acknowledgment; do not wait out a dead
        // worker's claim or run inference again in this recovery window.
        if existing_reply.is_none() && until.is_some_and(|t| t > now) {
            return Ok(None);
        }
        tx.execute(
            "UPDATE subscription_deliveries SET claimed_until = ?2 WHERE delivery_id = ?1",
            params![work_id, now + CLAIM_SECONDS],
        )?;
        tx.commit()?;
        Ok(Some(ActorWork {
            reply_message_id: format!("actor-reply:{work_id}"),
            work_id,
            room_id,
            message_id,
            message_seq,
            input,
            existing_reply,
        }))
    }

    /// Only claimed work can finish. Replied requires its durable reply; explicit
    /// skipped/failed outcomes allow poison messages to stop blocking the actor.
    pub fn complete_actor_work(
        &self,
        id: &str,
        work_id: &str,
        agent_id: &str,
        outcome: cowchat_core::ActorWorkOutcome,
    ) -> Result<(), StoreError> {
        use cowchat_core::ActorWorkOutcome;
        let status = match outcome {
            ActorWorkOutcome::Replied => "completed",
            ActorWorkOutcome::Skipped => "skipped",
            ActorWorkOutcome::Failed => "failed",
        };
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let work: Option<(i64, String, bool)> = tx.query_row(
            "SELECT w.message_seq, w.status,
                EXISTS(SELECT 1 FROM messages m WHERE m.message_id = 'actor-reply:' || w.delivery_id
                       AND m.agent_id = a.agent_id AND m.room_id = s.room_id AND m.reply_to_message = w.message_id)
             FROM subscription_deliveries w JOIN actor_subscriptions a USING(subscription_id)
             JOIN subscriptions s USING(subscription_id)
             WHERE w.delivery_id = ?1 AND w.subscription_id = ?2 AND a.agent_id = ?3
               AND s.status = 'active' AND w.claimed_until IS NOT NULL
               AND NOT EXISTS(SELECT 1 FROM subscription_deliveries earlier WHERE earlier.subscription_id = w.subscription_id
                              AND earlier.status = 'pending' AND earlier.message_seq < w.message_seq)",
            params![work_id, id, agent_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional()?;
        let Some((seq, previous, has_reply)) = work else {
            return Err(StoreError::InvalidActorWork);
        };
        if (previous != "pending" && previous != status)
            || (matches!(outcome, ActorWorkOutcome::Replied) && !has_reply)
            || (!matches!(outcome, ActorWorkOutcome::Replied) && has_reply)
        {
            return Err(StoreError::InvalidActorWork);
        }
        tx.execute(
            "UPDATE subscription_deliveries SET status = ?2 WHERE delivery_id = ?1",
            params![work_id, status],
        )?;
        tx.execute("UPDATE actor_subscriptions SET processed_seq = MAX(processed_seq, ?2) WHERE subscription_id = ?1", params![id, seq])?;
        tx.commit()?;
        Ok(())
    }
}
