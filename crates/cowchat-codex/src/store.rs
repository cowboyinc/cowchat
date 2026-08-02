use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::Path;
use std::sync::Mutex;

pub struct WakeStore {
    connection: Mutex<Connection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reservation {
    pub duplicate: bool,
    pub room_seq: Option<i64>,
}

pub struct EventReservation<'a> {
    pub target: &'a str,
    pub source: &'a str,
    pub event_id: &'a str,
    pub event_json: &'a str,
    pub room_id: &'a str,
    pub wake_hint_rank: i64,
    pub now_unix: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AckState {
    pub last_acked_seq: i64,
    pub max_read_seq: i64,
    pub max_pending_seq: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveredEvent {
    pub target: String,
    pub source: String,
    pub event_id: String,
    pub event_json: String,
    pub room_id: String,
    pub room_seq: i64,
    pub wake_hint_rank: i64,
}

impl WakeStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(StoreError::CreateParent)?;
        }
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self, StoreError> {
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS wake_events (
                 target       TEXT NOT NULL,
                 source       TEXT NOT NULL,
                 event_id     TEXT NOT NULL,
                 event_json   TEXT NOT NULL,
                 room_id      TEXT NOT NULL,
                 wake_hint_rank INTEGER NOT NULL,
                 room_seq     INTEGER,
                 message_id   TEXT,
                 created_at   INTEGER NOT NULL,
                 PRIMARY KEY (target, source, event_id)
             );
             CREATE INDEX IF NOT EXISTS idx_wake_events_target_seq
                 ON wake_events(target, room_seq);

             CREATE TABLE IF NOT EXISTS wake_target_state (
                 target           TEXT PRIMARY KEY,
                 last_acked_seq   INTEGER NOT NULL DEFAULT 0,
                 max_read_seq     INTEGER NOT NULL DEFAULT 0,
                 wake_claimed_at  INTEGER
             );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn reserve_event(&self, event: EventReservation<'_>) -> Result<Reservation, StoreError> {
        let EventReservation {
            target,
            source,
            event_id,
            event_json,
            room_id,
            wake_hint_rank,
            now_unix,
        } = event;
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO wake_target_state(target) VALUES (?1)",
            [target],
        )?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO wake_events
                 (target, source, event_id, event_json, room_id, wake_hint_rank, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                target,
                source,
                event_id,
                event_json,
                room_id,
                wake_hint_rank,
                now_unix
            ],
        )?;
        if inserted == 1 {
            tx.commit()?;
            return Ok(Reservation {
                duplicate: false,
                room_seq: None,
            });
        }

        let existing: (String, String, i64, Option<i64>) = tx.query_row(
            "SELECT event_json, room_id, wake_hint_rank, room_seq FROM wake_events
             WHERE target = ?1 AND source = ?2 AND event_id = ?3",
            params![target, source, event_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        if existing.0 != event_json || existing.1 != room_id || existing.2 != wake_hint_rank {
            return Err(StoreError::IdempotencyConflict {
                target: target.to_string(),
                event_source: source.to_string(),
                event_id: event_id.to_string(),
            });
        }
        tx.commit()?;
        Ok(Reservation {
            duplicate: true,
            room_seq: existing.3,
        })
    }

    pub fn mark_delivered(
        &self,
        target: &str,
        source: &str,
        event_id: &str,
        room_seq: i64,
        message_id: &str,
    ) -> Result<(), StoreError> {
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        let changed = connection.execute(
            "UPDATE wake_events SET room_seq = ?4, message_id = ?5
             WHERE target = ?1 AND source = ?2 AND event_id = ?3
               AND (room_seq IS NULL OR room_seq = ?4)",
            params![target, source, event_id, room_seq, message_id],
        )?;
        if changed != 1 {
            return Err(StoreError::MissingReservation);
        }
        Ok(())
    }

    pub fn claim_wake(
        &self,
        target: &str,
        observed_seq: i64,
        now_unix: i64,
        lease_seconds: i64,
    ) -> Result<bool, StoreError> {
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        connection.execute(
            "INSERT OR IGNORE INTO wake_target_state(target) VALUES (?1)",
            [target],
        )?;
        let changed = connection.execute(
            "UPDATE wake_target_state
             SET wake_claimed_at = ?3
             WHERE target = ?1
               AND last_acked_seq < ?2
               AND (wake_claimed_at IS NULL OR wake_claimed_at <= ?3 - ?4)",
            params![target, observed_seq, now_unix, lease_seconds],
        )?;
        Ok(changed == 1)
    }

    pub fn release_wake(&self, target: &str) -> Result<(), StoreError> {
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        connection.execute(
            "UPDATE wake_target_state SET wake_claimed_at = NULL WHERE target = ?1",
            [target],
        )?;
        Ok(())
    }

    pub fn last_acked_seq(&self, target: &str) -> Result<i64, StoreError> {
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        Ok(connection
            .query_row(
                "SELECT last_acked_seq FROM wake_target_state WHERE target = ?1",
                [target],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    pub fn record_read(&self, target: &str, max_seq: i64) -> Result<AckState, StoreError> {
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        connection.execute(
            "INSERT INTO wake_target_state(target, max_read_seq)
             VALUES (?1, ?2)
             ON CONFLICT(target) DO UPDATE SET
                 max_read_seq = MAX(max_read_seq, excluded.max_read_seq)",
            params![target, max_seq],
        )?;
        Self::load_ack_state(&connection, target)
    }

    pub fn acknowledge(&self, target: &str, cursor: i64) -> Result<AckState, StoreError> {
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = Self::load_ack_state(&tx, target)?;
        if cursor > state.max_read_seq {
            return Err(StoreError::AckBeyondRead {
                cursor,
                max_read_seq: state.max_read_seq,
            });
        }
        tx.execute(
            "UPDATE wake_target_state
             SET last_acked_seq = MAX(last_acked_seq, ?2), wake_claimed_at = NULL
             WHERE target = ?1",
            params![target, cursor],
        )?;
        let updated = Self::load_ack_state(&tx, target)?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn max_pending_seq(&self, target: &str) -> Result<Option<i64>, StoreError> {
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        let last_acked = self.last_acked_seq_locked(&connection, target)?;
        Ok(connection.query_row(
            "SELECT MAX(room_seq) FROM wake_events
             WHERE target = ?1 AND room_seq > ?2",
            params![target, last_acked],
            |row| row.get(0),
        )?)
    }

    pub fn max_pending_eligible_seq(
        &self,
        target: &str,
        min_wake_hint_rank: i64,
    ) -> Result<Option<i64>, StoreError> {
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        let last_acked = self.last_acked_seq_locked(&connection, target)?;
        Ok(connection.query_row(
            "SELECT MAX(room_seq) FROM wake_events
             WHERE target = ?1 AND room_seq > ?2 AND wake_hint_rank >= ?3",
            params![target, last_acked, min_wake_hint_rank],
            |row| row.get(0),
        )?)
    }

    pub fn delivered_event(
        &self,
        target: &str,
        room_seq: i64,
    ) -> Result<Option<DeliveredEvent>, StoreError> {
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        connection
            .query_row(
                "SELECT target, source, event_id, event_json, room_id, room_seq, wake_hint_rank
                 FROM wake_events WHERE target = ?1 AND room_seq = ?2",
                params![target, room_seq],
                |row| {
                    Ok(DeliveredEvent {
                        target: row.get(0)?,
                        source: row.get(1)?,
                        event_id: row.get(2)?,
                        event_json: row.get(3)?,
                        room_id: row.get(4)?,
                        room_seq: row.get(5)?,
                        wake_hint_rank: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    fn load_ack_state(connection: &Connection, target: &str) -> Result<AckState, StoreError> {
        let (last_acked_seq, max_read_seq) = connection
            .query_row(
                "SELECT last_acked_seq, max_read_seq FROM wake_target_state WHERE target = ?1",
                [target],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .unwrap_or((0, 0));
        let max_pending_seq = connection.query_row(
            "SELECT MAX(room_seq) FROM wake_events
             WHERE target = ?1 AND room_seq > ?2",
            params![target, last_acked_seq],
            |row| row.get(0),
        )?;
        Ok(AckState {
            last_acked_seq,
            max_read_seq,
            max_pending_seq,
        })
    }

    fn last_acked_seq_locked(
        &self,
        connection: &Connection,
        target: &str,
    ) -> Result<i64, StoreError> {
        Ok(connection
            .query_row(
                "SELECT last_acked_seq FROM wake_target_state WHERE target = ?1",
                [target],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("failed to create wake database directory: {0}")]
    CreateParent(#[source] std::io::Error),
    #[error("wake database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "event id was reused with different content: target={target} source={event_source} id={event_id}"
    )]
    IdempotencyConflict {
        target: String,
        event_source: String,
        event_id: String,
    },
    #[error("wake event reservation disappeared before delivery was recorded")]
    MissingReservation,
    #[error(
        "cannot acknowledge cursor {cursor}; highest cursor returned by read is {max_read_seq}"
    )]
    AckBeyondRead { cursor: i64, max_read_seq: i64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_reservation(event_json: &str, now_unix: i64) -> EventReservation<'_> {
        EventReservation {
            target: "reviewer",
            source: "ci",
            event_id: "evt-1",
            event_json,
            room_id: "room",
            wake_hint_rank: 1,
            now_unix,
        }
    }

    #[test]
    fn deduplicates_exact_event_and_rejects_conflicting_reuse() {
        let store = WakeStore::open_in_memory().unwrap();
        let first = store
            .reserve_event(test_reservation("{\"ok\":true}", 1))
            .unwrap();
        assert_eq!(
            first,
            Reservation {
                duplicate: false,
                room_seq: None
            }
        );
        store
            .mark_delivered("reviewer", "ci", "evt-1", 7, "msg-7")
            .unwrap();
        let duplicate = store
            .reserve_event(test_reservation("{\"ok\":true}", 2))
            .unwrap();
        assert_eq!(duplicate.room_seq, Some(7));
        assert!(duplicate.duplicate);

        let conflict = store.reserve_event(test_reservation("{\"ok\":false}", 3));
        assert!(matches!(
            conflict,
            Err(StoreError::IdempotencyConflict { .. })
        ));
    }

    #[test]
    fn wake_claim_is_leased_and_ack_cannot_skip_unread_events() {
        let store = WakeStore::open_in_memory().unwrap();
        store.reserve_event(test_reservation("{}", 1)).unwrap();
        store
            .mark_delivered("reviewer", "ci", "evt-1", 7, "msg-7")
            .unwrap();
        assert!(store.claim_wake("reviewer", 7, 100, 30).unwrap());
        assert!(!store.claim_wake("reviewer", 7, 110, 30).unwrap());
        assert!(store.claim_wake("reviewer", 7, 131, 30).unwrap());

        assert!(matches!(
            store.acknowledge("reviewer", 7),
            Err(StoreError::AckBeyondRead { .. })
        ));
        store.record_read("reviewer", 7).unwrap();
        let ack = store.acknowledge("reviewer", 7).unwrap();
        assert_eq!(ack.last_acked_seq, 7);
        assert_eq!(ack.max_pending_seq, None);
        assert!(!store.claim_wake("reviewer", 7, 200, 30).unwrap());
    }
}
