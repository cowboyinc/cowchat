//! Local revision guards for outcomes from asynchronous webhook attempts.
use super::*;

pub(crate) enum WebhookOutcome<'a> {
    Complete,
    Abandon {
        reason: &'a str,
        fail_subscription: bool,
    },
    Retry {
        reason: &'a str,
        next: DateTime<Utc>,
    },
}

impl Store {
    pub(crate) fn webhook_attempt_current(
        &self,
        delivery: &PendingDelivery,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        attempt_current_on(&conn, delivery)
    }

    pub(crate) fn finish_webhook_attempt(
        &self,
        delivery: &PendingDelivery,
        outcome: WebhookOutcome<'_>,
    ) -> Result<bool, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !attempt_current_on(&tx, delivery)? {
            return Ok(false);
        }
        match outcome {
            WebhookOutcome::Complete => {
                tx.execute("UPDATE subscriptions SET last_delivered_seq=MAX(last_delivered_seq,?2),failure_count=0
                    WHERE subscription_id=?1", params![delivery.subscription_id,delivery.message_seq])?;
                tx.execute(
                    "DELETE FROM subscription_deliveries WHERE delivery_id=?1",
                    [&delivery.delivery_id],
                )?;
            }
            WebhookOutcome::Abandon {
                reason,
                fail_subscription,
            } => {
                tx.execute("UPDATE subscription_deliveries SET status='abandoned',last_error=?2 WHERE delivery_id=?1",
                    params![delivery.delivery_id,reason])?;
                if fail_subscription {
                    tx.execute("UPDATE subscriptions SET status='failed',failure_count=?2 WHERE subscription_id=?1",
                        params![delivery.subscription_id,delivery.attempts+1])?;
                }
            }
            WebhookOutcome::Retry { reason, next } => {
                tx.execute("UPDATE subscription_deliveries SET next_attempt_at=?2,attempts=?3,last_error=?4 WHERE delivery_id=?1",
                    params![delivery.delivery_id,next.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),delivery.attempts+1,reason])?;
            }
        }
        tx.commit()?;
        Ok(true)
    }
}

fn attempt_current_on(conn: &Connection, d: &PendingDelivery) -> Result<bool, StoreError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM subscription_deliveries d
        JOIN subscriptions s ON s.subscription_id=d.subscription_id
        WHERE d.delivery_id=?1 AND d.subscription_id=?2 AND s.revision=?3
        AND d.status='pending' AND d.attempts=?4)",
        params![d.delivery_id, d.subscription_id, d.revision, d.attempts],
        |r| r.get(0),
    )?)
}
