use fs2::FileExt;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

pub struct WakeStore {
    connection: Mutex<Connection>,
    scope: String,
    lock_dir: Option<PathBuf>,
    process_locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
}

pub struct TargetLockGuard {
    file: Option<File>,
    _process_guard: Option<OwnedMutexGuard<()>>,
}

static PROCESS_FILE_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<AsyncMutex<()>>>>> =
    OnceLock::new();

impl Drop for TargetLockGuard {
    fn drop(&mut self) {
        if let Some(file) = &self.file {
            let _ = FileExt::unlock(file);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetHandle {
    pub identity: String,
    pub state_id: String,
    pub alias: String,
    pub room_id: String,
    pub floor_seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reservation {
    pub duplicate: bool,
    pub room_seq: Option<i64>,
    pub event_json: String,
    pub event_digest: String,
    pub legacy_metadata: bool,
}

pub struct EventReservation<'a> {
    pub source: &'a str,
    pub event_id: &'a str,
    /// Canonical caller-controlled fields. Generated time is deliberately not
    /// part of this value, so a retry that omitted time remains idempotent.
    pub request_json: &'a str,
    /// Complete event persisted and delivered for a first reservation.
    pub event_json: &'a str,
    pub event_digest: &'a str,
    pub room_id: &'a str,
    pub wake_hint_rank: i64,
    pub now_unix: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WakeClaim {
    pub generation: i64,
    pub claimed_seq: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryClaim {
    Delivered(i64),
    Claimed { generation: i64 },
    InFlight,
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
    pub event_digest: String,
    pub room_id: String,
    pub room_seq: i64,
    pub wake_hint_rank: i64,
    pub legacy_metadata: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyPolicy {
    Reject,
    AllowMaintenance,
}

impl WakeStore {
    pub fn open(path: &Path, scope: &str) -> Result<Self, StoreError> {
        Self::open_with_legacy_policy(path, scope, LegacyPolicy::Reject)
    }

    /// Open a database that may contain pre-v0.7 unscoped state. Callers must
    /// use this only for an explicit, config-bound migration or reset command;
    /// normal bridge processes fail closed instead of silently seeding past it.
    pub fn open_for_legacy_maintenance(path: &Path, scope: &str) -> Result<Self, StoreError> {
        Self::open_with_legacy_policy(path, scope, LegacyPolicy::AllowMaintenance)
    }

    fn open_with_legacy_policy(
        path: &Path,
        scope: &str,
        legacy_policy: LegacyPolicy,
    ) -> Result<Self, StoreError> {
        validate_scope(scope)?;
        if path.as_os_str().is_empty()
            || path
                .to_str()
                .is_some_and(|path| path == ":memory:" || path.starts_with("file:"))
        {
            return Err(StoreError::InvalidDatabasePath(path.to_path_buf()));
        }
        if let Some(parent) = path.parent() {
            create_private_directory(parent)?;
        }
        create_private_file(path)?;
        let path = canonical_database_path(path)?;
        let lock_dir = lock_directory_for(&path)?;
        create_private_directory(&lock_dir)?;
        set_private_directory_permissions(&lock_dir)?;
        set_private_file_permissions(&path)?;
        let sidecars = [
            PathBuf::from(format!("{}-wal", path.display())),
            PathBuf::from(format!("{}-shm", path.display())),
        ];
        for sidecar in &sidecars {
            if sidecar.exists() {
                set_private_file_permissions(sidecar)?;
            }
        }
        let connection = Connection::open(&path)?;
        let store = Self::from_connection(connection, scope, Some(lock_dir), legacy_policy)?;
        for sidecar in &sidecars {
            if sidecar.exists() {
                set_private_file_permissions(sidecar)?;
            }
        }
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::from_connection(
            Connection::open_in_memory()?,
            "test-scope",
            None,
            LegacyPolicy::Reject,
        )
    }

    pub fn open_in_memory_scoped(scope: &str) -> Result<Self, StoreError> {
        validate_scope(scope)?;
        Self::from_connection(
            Connection::open_in_memory()?,
            scope,
            None,
            LegacyPolicy::Reject,
        )
    }

    fn from_connection(
        connection: Connection,
        scope: &str,
        lock_dir: Option<PathBuf>,
        legacy_policy: LegacyPolicy,
    ) -> Result<Self, StoreError> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS wake_events_v2 (
                 scope         TEXT NOT NULL,
                 target        TEXT NOT NULL,
                 source        TEXT NOT NULL,
                 event_id      TEXT NOT NULL,
                 request_json  TEXT NOT NULL,
                 event_json    TEXT NOT NULL,
                 event_digest  TEXT NOT NULL,
                 room_id       TEXT NOT NULL,
                 wake_hint_rank INTEGER NOT NULL,
                 room_seq      INTEGER,
                 message_id    TEXT,
                 created_at    INTEGER NOT NULL,
                 delivery_claimed_at INTEGER,
                 delivery_generation INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (scope, target, source, event_id)
             );
             CREATE INDEX IF NOT EXISTS idx_wake_events_v2_target_seq
                 ON wake_events_v2(scope, target, room_seq);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_wake_events_v2_unique_room_seq
                 ON wake_events_v2(scope, target, room_seq) WHERE room_seq IS NOT NULL;

             CREATE TABLE IF NOT EXISTS wake_target_state_v2 (
                 scope             TEXT NOT NULL,
                 target            TEXT NOT NULL,
                 last_acked_seq    INTEGER NOT NULL DEFAULT 0,
                 max_read_seq      INTEGER NOT NULL DEFAULT 0,
                 wake_claimed_at   INTEGER,
                 wake_claimed_seq  INTEGER,
                 wake_generation   INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (scope, target)
             );

             CREATE TABLE IF NOT EXISTS wake_read_cursors_v2 (
                 scope       TEXT NOT NULL,
                 target      TEXT NOT NULL,
                 cursor      INTEGER NOT NULL CHECK(cursor >= 0),
                 PRIMARY KEY (scope, target, cursor)
             );

             CREATE TABLE IF NOT EXISTS wake_relay_state_v2 (
                 scope        TEXT NOT NULL,
                 target       TEXT NOT NULL,
                 room_id      TEXT NOT NULL,
                 cursor       INTEGER NOT NULL CHECK(cursor >= 0),
                 PRIMARY KEY (scope, target)
             );

             CREATE TABLE IF NOT EXISTS wake_target_bindings_v2 (
                 scope           TEXT NOT NULL,
                 target_alias    TEXT NOT NULL,
                 target_identity TEXT NOT NULL,
                 room_id         TEXT NOT NULL,
                 state_id        TEXT NOT NULL,
                 floor_seq       INTEGER NOT NULL CHECK(floor_seq >= 0),
                 PRIMARY KEY (scope, target_alias),
                 UNIQUE (scope, state_id)
             );

             CREATE TABLE IF NOT EXISTS wake_legacy_events_v2 (
                 scope      TEXT NOT NULL,
                 target     TEXT NOT NULL,
                 source     TEXT NOT NULL,
                 event_id   TEXT NOT NULL,
                 PRIMARY KEY (scope, target, source, event_id)
             );",
        )?;
        let legacy_targets = legacy_targets(&connection)?;
        if legacy_policy == LegacyPolicy::Reject && !legacy_targets.is_empty() {
            return Err(StoreError::LegacyStatePresent(legacy_targets));
        }
        Ok(Self {
            connection: Mutex::new(connection),
            scope: scope.to_string(),
            lock_dir,
            process_locks: Mutex::new(HashMap::new()),
        })
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub fn current_target(
        &self,
        identity: &str,
        alias: &str,
        room_id: &str,
    ) -> Result<Option<TargetHandle>, StoreError> {
        validate_target(identity, alias, room_id, 0)?;
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        let Some((stored_identity, stored_room, state_id, floor_seq)) =
            load_binding(&connection, &self.scope, alias)?
        else {
            return Ok(None);
        };
        if stored_identity != identity {
            return Ok(None);
        }
        if stored_room != room_id {
            return Err(StoreError::TargetBindingRoomMismatch {
                alias: alias.to_string(),
                stored_room,
                configured_room: room_id.to_string(),
            });
        }
        let handle = TargetHandle {
            identity: identity.to_string(),
            state_id,
            alias: alias.to_string(),
            room_id: room_id.to_string(),
            floor_seq,
        };
        assert_current(&connection, &self.scope, &handle)?;
        Ok(Some(handle))
    }

    pub fn lock_target_shared(&self, alias: &str) -> Result<TargetLockGuard, StoreError> {
        self.lock_target(alias, false)
    }

    pub fn lock_target_exclusive(&self, alias: &str) -> Result<TargetLockGuard, StoreError> {
        self.lock_target(alias, true)
    }

    /// Acquire the process and filesystem target fence without parking a Tokio
    /// worker. Production async call sites must use this form; the synchronous
    /// methods above are retained for non-async diagnostics and focused tests.
    pub async fn lock_target_exclusive_async(
        &self,
        alias: &str,
    ) -> Result<TargetLockGuard, StoreError> {
        if alias.trim().is_empty() {
            return Err(StoreError::InvalidTargetAlias);
        }
        let lock_path = self
            .lock_dir
            .as_ref()
            .map(|lock_dir| target_lock_path(lock_dir, &self.scope, alias));
        let process_lock = if let Some(path) = &lock_path {
            process_file_lock(path)
        } else {
            let mut locks = self
                .process_locks
                .lock()
                .expect("wake process-lock map mutex poisoned");
            locks
                .entry(alias.to_string())
                .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                .clone()
        };
        let process_guard = process_lock.lock_owned().await;
        let Some(path) = lock_path else {
            return Ok(TargetLockGuard {
                file: None,
                _process_guard: Some(process_guard),
            });
        };
        let task_path = path.clone();
        let file = tokio::task::spawn_blocking(move || {
            let file = open_private_lock_file(&task_path)?;
            FileExt::lock_exclusive(&file)
                .map_err(|source| StoreError::AcquireTargetLock(task_path, source))?;
            Ok::<_, StoreError>(file)
        })
        .await
        .map_err(StoreError::TargetLockTask)??;
        Ok(TargetLockGuard {
            file: Some(file),
            _process_guard: Some(process_guard),
        })
    }

    fn lock_target(&self, alias: &str, exclusive: bool) -> Result<TargetLockGuard, StoreError> {
        if alias.trim().is_empty() {
            return Err(StoreError::InvalidTargetAlias);
        }
        let Some(lock_dir) = &self.lock_dir else {
            return Ok(TargetLockGuard {
                file: None,
                _process_guard: None,
            });
        };
        let path = target_lock_path(lock_dir, &self.scope, alias);
        let file = open_private_lock_file(&path)?;
        if exclusive {
            FileExt::lock_exclusive(&file)
        } else {
            FileExt::lock_shared(&file)
        }
        .map_err(|source| StoreError::AcquireTargetLock(path, source))?;
        Ok(TargetLockGuard {
            file: Some(file),
            _process_guard: None,
        })
    }

    pub fn activate_target(
        &self,
        identity: &str,
        alias: &str,
        room_id: &str,
        room_tip: i64,
    ) -> Result<TargetHandle, StoreError> {
        validate_target(identity, alias, room_id, room_tip)?;
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some((stored_identity, stored_room, state_id, floor_seq)) =
            load_binding(&tx, &self.scope, alias)?
        {
            if stored_identity != identity {
                let handle = rotate_target_state(
                    &tx,
                    &self.scope,
                    identity,
                    alias,
                    room_id,
                    room_tip,
                    Some(&state_id),
                )?;
                tx.commit()?;
                return Ok(handle);
            }
            if stored_room != room_id {
                return Err(StoreError::TargetBindingRoomMismatch {
                    alias: alias.to_string(),
                    stored_room,
                    configured_room: room_id.to_string(),
                });
            }
            let last_acked_seq: i64 = tx
                .query_row(
                    "SELECT last_acked_seq FROM wake_target_state_v2
                     WHERE scope = ?1 AND target = ?2",
                    params![state_id, alias],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or_else(|| StoreError::MissingTargetState(alias.to_string()))?;
            let max_delivered_seq: i64 = tx.query_row(
                "SELECT COALESCE(MAX(room_seq), 0) FROM wake_events_v2
                 WHERE scope = ?1 AND target = ?2",
                params![state_id, alias],
                |row| row.get(0),
            )?;
            let required_seq = floor_seq.max(last_acked_seq).max(max_delivered_seq);
            if room_tip < required_seq {
                return Err(StoreError::RoomTipBehindCursor {
                    room_tip,
                    floor_seq,
                    last_acked_seq,
                    max_delivered_seq,
                });
            }
            let handle = TargetHandle {
                identity: identity.to_string(),
                state_id,
                alias: alias.to_string(),
                room_id: room_id.to_string(),
                floor_seq,
            };
            tx.commit()?;
            return Ok(handle);
        }

        let state_id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO wake_target_bindings_v2
                 (scope, target_alias, target_identity, room_id, state_id, floor_seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![self.scope, alias, identity, room_id, state_id, room_tip],
        )?;
        tx.execute(
            "INSERT INTO wake_target_state_v2
                 (scope, target, last_acked_seq, max_read_seq)
             VALUES (?1, ?2, ?3, ?3)",
            params![state_id, alias, room_tip],
        )?;
        let handle = TargetHandle {
            identity: identity.to_string(),
            state_id,
            alias: alias.to_string(),
            room_id: room_id.to_string(),
            floor_seq: room_tip,
        };
        tx.commit()?;
        Ok(handle)
    }

    pub fn reset_target(
        &self,
        identity: &str,
        alias: &str,
        room_id: &str,
        room_tip: i64,
    ) -> Result<TargetHandle, StoreError> {
        validate_target(identity, alias, room_id, room_tip)?;
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old_state_id = load_binding(&tx, &self.scope, alias)?.map(|binding| binding.2);
        let handle = rotate_target_state(
            &tx,
            &self.scope,
            identity,
            alias,
            room_id,
            room_tip,
            old_state_id.as_deref(),
        )?;
        tx.commit()?;
        Ok(handle)
    }

    /// Transactionally bind and migrate one unscoped v0.6 target using the
    /// operator's current configuration. Delivered-but-unacknowledged events
    /// retain their Cowchat sequence and are deliberately made unread again.
    pub fn migrate_legacy_target(
        &self,
        identity: &str,
        alias: &str,
        room_id: &str,
        room_tip: i64,
    ) -> Result<TargetHandle, StoreError> {
        validate_target(identity, alias, room_id, room_tip)?;
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if load_binding(&tx, &self.scope, alias)?.is_some() {
            return Err(StoreError::TargetAlreadyInitialized(alias.to_string()));
        }

        let legacy_state = if table_exists(&tx, "wake_target_state")? {
            tx.query_row(
                "SELECT last_acked_seq, max_read_seq FROM wake_target_state WHERE target = ?1",
                [alias],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
        } else {
            None
        };
        let legacy_events = if table_exists(&tx, "wake_events")? {
            let mut statement = tx.prepare(
                "SELECT source, event_id, event_json, room_id, wake_hint_rank,
                        room_seq, message_id, created_at
                 FROM wake_events WHERE target = ?1
                 ORDER BY CASE WHEN room_seq IS NULL THEN 1 ELSE 0 END, room_seq, source, event_id",
            )?;
            let events = statement
                .query_map([alias], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            events
        } else {
            Vec::new()
        };
        if legacy_state.is_none() && legacy_events.is_empty() {
            return Err(StoreError::LegacyTargetNotFound(alias.to_string()));
        }

        let (last_acked_seq, legacy_max_read_seq) = legacy_state.unwrap_or((0, 0));
        if last_acked_seq < 0
            || legacy_max_read_seq < last_acked_seq
            || legacy_events
                .iter()
                .filter_map(|event| event.5)
                .any(|seq| seq < 0)
        {
            return Err(StoreError::InvalidLegacyCursor(alias.to_string()));
        }
        if let Some(event) = legacy_events.iter().find(|event| event.3 != room_id) {
            return Err(StoreError::LegacyTargetRoomMismatch {
                alias: alias.to_string(),
                stored_room: event.3.clone(),
                configured_room: room_id.to_string(),
            });
        }
        let max_delivered_seq = legacy_events
            .iter()
            .filter_map(|event| event.5)
            .max()
            .unwrap_or(0);
        if room_tip < last_acked_seq.max(max_delivered_seq) {
            return Err(StoreError::RoomTipBehindCursor {
                room_tip,
                floor_seq: last_acked_seq,
                last_acked_seq,
                max_delivered_seq,
            });
        }

        let state_id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO wake_target_bindings_v2
                 (scope, target_alias, target_identity, room_id, state_id, floor_seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                self.scope,
                alias,
                identity,
                room_id,
                state_id,
                last_acked_seq
            ],
        )?;
        // Reset max_read_seq to the acknowledged cursor. v0.6 did not retain
        // exact returned cursors, so granting its broader max-read authority
        // would permit an acknowledgement the current generation never read.
        tx.execute(
            "INSERT INTO wake_target_state_v2
                 (scope, target, last_acked_seq, max_read_seq)
             VALUES (?1, ?2, ?3, ?3)",
            params![state_id, alias, last_acked_seq],
        )?;
        for (source, event_id, event_json, event_room, hint, seq, message_id, created_at) in
            legacy_events
        {
            let event_digest = format!("{:x}", Sha256::digest(event_json.as_bytes()));
            tx.execute(
                "INSERT INTO wake_events_v2
                     (scope, target, source, event_id, request_json, event_json,
                      event_digest, room_id, wake_hint_rank, room_seq, message_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    state_id,
                    alias,
                    source,
                    event_id,
                    event_json,
                    event_digest,
                    event_room,
                    hint,
                    seq,
                    message_id,
                    created_at
                ],
            )?;
            tx.execute(
                "INSERT INTO wake_legacy_events_v2(scope, target, source, event_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![state_id, alias, source, event_id],
            )?;
        }
        delete_legacy_target(&tx, alias)?;
        let handle = TargetHandle {
            identity: identity.to_string(),
            state_id,
            alias: alias.to_string(),
            room_id: room_id.to_string(),
            floor_seq: last_acked_seq,
        };
        tx.commit()?;
        Ok(handle)
    }

    /// Explicitly discard one target's legacy cursor/idempotency state before
    /// rotating it at a verified live room tip. This is the recovery path for
    /// corrupt or obsolete legacy state that cannot be migrated.
    pub fn reset_target_discarding_legacy(
        &self,
        identity: &str,
        alias: &str,
        room_id: &str,
        room_tip: i64,
    ) -> Result<TargetHandle, StoreError> {
        validate_target(identity, alias, room_id, room_tip)?;
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        delete_legacy_target(&tx, alias)?;
        let old_state_id = load_binding(&tx, &self.scope, alias)?.map(|binding| binding.2);
        let handle = rotate_target_state(
            &tx,
            &self.scope,
            identity,
            alias,
            room_id,
            room_tip,
            old_state_id.as_deref(),
        )?;
        tx.commit()?;
        Ok(handle)
    }

    pub fn reserve_event(
        &self,
        handle: &TargetHandle,
        event: EventReservation<'_>,
    ) -> Result<Reservation, StoreError> {
        if event.room_id != handle.room_id {
            return Err(StoreError::EventRoomMismatch {
                configured_room: handle.room_id.clone(),
                event_room: event.room_id.to_string(),
            });
        }
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_current(&tx, &self.scope, handle)?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO wake_events_v2
                 (scope, target, source, event_id, request_json, event_json,
                  event_digest, room_id, wake_hint_rank, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                handle.state_id,
                handle.alias,
                event.source,
                event.event_id,
                event.request_json,
                event.event_json,
                event.event_digest,
                event.room_id,
                event.wake_hint_rank,
                event.now_unix,
            ],
        )?;
        if inserted == 1 {
            tx.commit()?;
            return Ok(Reservation {
                duplicate: false,
                room_seq: None,
                event_json: event.event_json.to_string(),
                event_digest: event.event_digest.to_string(),
                legacy_metadata: false,
            });
        }

        let existing: (String, String, String, String, i64, Option<i64>, bool) = tx.query_row(
            "SELECT request_json, event_json, event_digest, room_id, wake_hint_rank, room_seq,
                    EXISTS(
                        SELECT 1 FROM wake_legacy_events_v2 legacy
                        WHERE legacy.scope = wake_events_v2.scope
                          AND legacy.target = wake_events_v2.target
                          AND legacy.source = wake_events_v2.source
                          AND legacy.event_id = wake_events_v2.event_id
                    )
             FROM wake_events_v2
             WHERE scope = ?1 AND target = ?2 AND source = ?3 AND event_id = ?4",
            params![handle.state_id, handle.alias, event.source, event.event_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )?;
        let same_request = if existing.6 {
            // v0.6 persisted only the completed event. Preserve its original
            // idempotency contract (exact event equality) after migration.
            existing.1 == event.event_json
        } else {
            existing.0 == event.request_json
        };
        if !same_request || existing.3 != event.room_id || existing.4 != event.wake_hint_rank {
            return Err(StoreError::IdempotencyConflict {
                target: handle.alias.clone(),
                event_source: event.source.to_string(),
                event_id: event.event_id.to_string(),
            });
        }
        tx.commit()?;
        Ok(Reservation {
            duplicate: true,
            room_seq: existing.5,
            event_json: existing.1,
            event_digest: existing.2,
            legacy_metadata: existing.6,
        })
    }

    pub fn claim_delivery(
        &self,
        handle: &TargetHandle,
        source: &str,
        event_id: &str,
        now_unix: i64,
        lease_seconds: i64,
    ) -> Result<DeliveryClaim, StoreError> {
        if lease_seconds <= 0 {
            return Err(StoreError::InvalidLeaseSeconds(lease_seconds));
        }
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_current(&tx, &self.scope, handle)?;
        let state: Option<(Option<i64>, Option<i64>, i64)> = tx
            .query_row(
                "SELECT room_seq, delivery_claimed_at, delivery_generation
                 FROM wake_events_v2
                 WHERE scope = ?1 AND target = ?2 AND source = ?3 AND event_id = ?4",
                params![handle.state_id, handle.alias, source, event_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((room_seq, claimed_at, generation)) = state else {
            return Err(StoreError::MissingReservation);
        };
        if let Some(seq) = room_seq {
            tx.commit()?;
            return Ok(DeliveryClaim::Delivered(seq));
        }
        if claimed_at.is_some_and(|claimed| claimed > now_unix - lease_seconds) {
            tx.commit()?;
            return Ok(DeliveryClaim::InFlight);
        }
        let next_generation = generation
            .checked_add(1)
            .ok_or(StoreError::GenerationOverflow)?;
        let changed = tx.execute(
            "UPDATE wake_events_v2
             SET delivery_claimed_at = ?5, delivery_generation = ?6
             WHERE scope = ?1 AND target = ?2 AND source = ?3 AND event_id = ?4
               AND room_seq IS NULL AND delivery_generation = ?7",
            params![
                handle.state_id,
                handle.alias,
                source,
                event_id,
                now_unix,
                next_generation,
                generation
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::LostClaimRace);
        }
        tx.commit()?;
        Ok(DeliveryClaim::Claimed {
            generation: next_generation,
        })
    }

    pub fn renew_delivery_claim(
        &self,
        handle: &TargetHandle,
        source: &str,
        event_id: &str,
        generation: i64,
        now_unix: i64,
    ) -> Result<bool, StoreError> {
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_current(&tx, &self.scope, handle)?;
        let renewed = tx.execute(
            "UPDATE wake_events_v2 SET delivery_claimed_at = ?6
             WHERE scope = ?1 AND target = ?2 AND source = ?3 AND event_id = ?4
               AND room_seq IS NULL AND delivery_generation = ?5
               AND delivery_claimed_at IS NOT NULL",
            params![
                handle.state_id,
                handle.alias,
                source,
                event_id,
                generation,
                now_unix
            ],
        )? == 1;
        tx.commit()?;
        Ok(renewed)
    }

    #[cfg(test)]
    pub fn expire_delivery_for_test(
        &self,
        handle: &TargetHandle,
        source: &str,
        event_id: &str,
    ) -> Result<(), StoreError> {
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_current(&tx, &self.scope, handle)?;
        let changed = tx.execute(
            "UPDATE wake_events_v2 SET delivery_claimed_at = 0
             WHERE scope = ?1 AND target = ?2 AND source = ?3 AND event_id = ?4
               AND room_seq IS NULL",
            params![handle.state_id, handle.alias, source, event_id],
        )?;
        if changed != 1 {
            return Err(StoreError::MissingReservation);
        }
        tx.commit()?;
        Ok(())
    }

    pub fn mark_delivered(
        &self,
        handle: &TargetHandle,
        source: &str,
        event_id: &str,
        generation: i64,
        room_seq: i64,
        message_id: &str,
    ) -> Result<(), StoreError> {
        if room_seq < 0 {
            return Err(StoreError::InvalidDeliveredSequence(room_seq));
        }
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_current(&tx, &self.scope, handle)?;
        let last_acked_seq = self.last_acked_seq_locked(&tx, handle)?;
        if room_seq <= last_acked_seq {
            return Err(StoreError::DeliveryBehindCursor {
                room_seq,
                last_acked_seq,
            });
        }
        let changed = tx.execute(
            "UPDATE wake_events_v2
             SET room_seq = ?6, message_id = ?7, delivery_claimed_at = NULL
             WHERE scope = ?1 AND target = ?2 AND source = ?3 AND event_id = ?4
               AND room_seq IS NULL AND delivery_generation = ?5",
            params![
                handle.state_id,
                handle.alias,
                source,
                event_id,
                generation,
                room_seq,
                message_id
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::StaleDeliveryClaim);
        }
        tx.commit()?;
        Ok(())
    }

    pub fn release_delivery(
        &self,
        handle: &TargetHandle,
        source: &str,
        event_id: &str,
        generation: i64,
    ) -> Result<bool, StoreError> {
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_current(&tx, &self.scope, handle)?;
        let released = tx.execute(
            "UPDATE wake_events_v2 SET delivery_claimed_at = NULL
             WHERE scope = ?1 AND target = ?2 AND source = ?3 AND event_id = ?4
               AND room_seq IS NULL AND delivery_generation = ?5",
            params![handle.state_id, handle.alias, source, event_id, generation],
        )? == 1;
        tx.commit()?;
        Ok(released)
    }

    pub fn claim_wake(
        &self,
        handle: &TargetHandle,
        observed_seq: i64,
        now_unix: i64,
        lease_seconds: i64,
    ) -> Result<Option<WakeClaim>, StoreError> {
        if lease_seconds <= 0 {
            return Err(StoreError::InvalidLeaseSeconds(lease_seconds));
        }
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_current(&tx, &self.scope, handle)?;
        let generation: i64 = tx.query_row(
            "SELECT wake_generation FROM wake_target_state_v2 WHERE scope = ?1 AND target = ?2",
            params![handle.state_id, handle.alias],
            |row| row.get(0),
        )?;
        let next_generation = generation
            .checked_add(1)
            .ok_or(StoreError::GenerationOverflow)?;
        let changed = tx.execute(
            "UPDATE wake_target_state_v2
             SET wake_claimed_at = ?4, wake_claimed_seq = ?3, wake_generation = ?6
             WHERE scope = ?1 AND target = ?2
               AND last_acked_seq < ?3
               AND (wake_claimed_at IS NULL OR wake_claimed_at <= ?4 - ?5)",
            params![
                handle.state_id,
                handle.alias,
                observed_seq,
                now_unix,
                lease_seconds,
                next_generation
            ],
        )?;
        tx.commit()?;
        Ok((changed == 1).then_some(WakeClaim {
            generation: next_generation,
            claimed_seq: observed_seq,
        }))
    }

    pub fn release_wake(
        &self,
        handle: &TargetHandle,
        claim: WakeClaim,
    ) -> Result<bool, StoreError> {
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_current(&tx, &self.scope, handle)?;
        let released = tx.execute(
            "UPDATE wake_target_state_v2
             SET wake_claimed_at = NULL, wake_claimed_seq = NULL
             WHERE scope = ?1 AND target = ?2 AND wake_generation = ?3 AND wake_claimed_seq = ?4",
            params![
                handle.state_id,
                handle.alias,
                claim.generation,
                claim.claimed_seq
            ],
        )? == 1;
        tx.commit()?;
        Ok(released)
    }

    #[cfg(test)]
    pub fn expire_wake_for_test(&self, handle: &TargetHandle) -> Result<(), StoreError> {
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_current(&tx, &self.scope, handle)?;
        tx.execute(
            "UPDATE wake_target_state_v2 SET wake_claimed_at = NULL
             WHERE scope = ?1 AND target = ?2",
            params![handle.state_id, handle.alias],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn last_acked_seq(&self, handle: &TargetHandle) -> Result<i64, StoreError> {
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        assert_current(&connection, &self.scope, handle)?;
        self.last_acked_seq_locked(&connection, handle)
    }

    pub fn authorize_read_cursor(
        &self,
        handle: &TargetHandle,
        cursor: i64,
    ) -> Result<(), StoreError> {
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        assert_current(&connection, &self.scope, handle)?;
        let last_acked = self.last_acked_seq_locked(&connection, handle)?;
        if cursor == last_acked {
            return Ok(());
        }
        let allowed = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM wake_read_cursors_v2
             WHERE scope = ?1 AND target = ?2 AND cursor = ?3)",
            params![handle.state_id, handle.alias, cursor],
            |row| row.get::<_, bool>(0),
        )?;
        if allowed {
            Ok(())
        } else {
            Err(StoreError::UnauthorizedReadCursor { cursor, last_acked })
        }
    }

    /// Record only rows actually returned to the caller. An empty result never
    /// grants authority over the caller-provided `after_cursor`.
    pub fn record_read(
        &self,
        handle: &TargetHandle,
        returned: &[i64],
    ) -> Result<AckState, StoreError> {
        validate_returned_cursors(returned)?;
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_current(&tx, &self.scope, handle)?;
        let before = self.load_ack_state(&tx, handle)?;
        for cursor in returned {
            let delivered = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM wake_events_v2
                 WHERE scope = ?1 AND target = ?2 AND room_seq = ?3)",
                params![handle.state_id, handle.alias, cursor],
                |row| row.get::<_, bool>(0),
            )?;
            if !delivered {
                return Err(StoreError::ReadCursorNotDelivered(*cursor));
            }
            tx.execute(
                "INSERT OR IGNORE INTO wake_read_cursors_v2(scope, target, cursor)
                 VALUES (?1, ?2, ?3)",
                params![handle.state_id, handle.alias, cursor],
            )?;
            tx.execute(
                "UPDATE wake_target_state_v2 SET max_read_seq = MAX(max_read_seq, ?3)
                 WHERE scope = ?1 AND target = ?2",
                params![handle.state_id, handle.alias, cursor],
            )?;
        }
        if let Some(through) = returned.last() {
            ensure_pending_range_read(&tx, handle, before.last_acked_seq, *through)?;
        }
        let state = self.load_ack_state(&tx, handle)?;
        tx.commit()?;
        Ok(state)
    }

    pub fn acknowledge(&self, handle: &TargetHandle, cursor: i64) -> Result<AckState, StoreError> {
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_current(&tx, &self.scope, handle)?;
        let state = self.load_ack_state(&tx, handle)?;
        if cursor < state.last_acked_seq {
            return Err(StoreError::AckRegressed {
                cursor,
                last_acked_seq: state.last_acked_seq,
            });
        }
        if cursor > state.max_read_seq {
            return Err(StoreError::AckBeyondRead {
                cursor,
                max_read_seq: state.max_read_seq,
            });
        }
        if cursor > state.last_acked_seq {
            let authorized = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM wake_read_cursors_v2
                 WHERE scope = ?1 AND target = ?2 AND cursor = ?3)",
                params![handle.state_id, handle.alias, cursor],
                |row| row.get::<_, bool>(0),
            )?;
            if !authorized {
                return Err(StoreError::AckCursorNotReturned(cursor));
            }
            ensure_pending_range_read(&tx, handle, state.last_acked_seq, cursor)?;
            tx.execute(
                "UPDATE wake_target_state_v2
                 SET last_acked_seq = ?3,
                     wake_claimed_at = CASE WHEN wake_claimed_seq <= ?3 THEN NULL ELSE wake_claimed_at END,
                     wake_claimed_seq = CASE WHEN wake_claimed_seq <= ?3 THEN NULL ELSE wake_claimed_seq END
                 WHERE scope = ?1 AND target = ?2",
                params![handle.state_id, handle.alias, cursor],
            )?;
            tx.execute(
                "DELETE FROM wake_read_cursors_v2
                 WHERE scope = ?1 AND target = ?2 AND cursor <= ?3",
                params![handle.state_id, handle.alias, cursor],
            )?;
        }
        let updated = self.load_ack_state(&tx, handle)?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn max_pending_seq(&self, handle: &TargetHandle) -> Result<Option<i64>, StoreError> {
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        assert_current(&connection, &self.scope, handle)?;
        let last_acked = self.last_acked_seq_locked(&connection, handle)?;
        Ok(connection.query_row(
            "SELECT MAX(room_seq) FROM wake_events_v2
             WHERE scope = ?1 AND target = ?2 AND room_seq > ?3",
            params![handle.state_id, handle.alias, last_acked],
            |row| row.get(0),
        )?)
    }

    pub fn max_pending_eligible_seq(
        &self,
        handle: &TargetHandle,
        min_wake_hint_rank: i64,
    ) -> Result<Option<i64>, StoreError> {
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        assert_current(&connection, &self.scope, handle)?;
        let last_acked = self.last_acked_seq_locked(&connection, handle)?;
        Ok(connection.query_row(
            "SELECT MAX(room_seq) FROM wake_events_v2
             WHERE scope = ?1 AND target = ?2 AND room_seq > ?3 AND wake_hint_rank >= ?4",
            params![
                handle.state_id,
                handle.alias,
                last_acked,
                min_wake_hint_rank
            ],
            |row| row.get(0),
        )?)
    }

    pub fn delivered_event(
        &self,
        handle: &TargetHandle,
        room_seq: i64,
    ) -> Result<Option<DeliveredEvent>, StoreError> {
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        assert_current(&connection, &self.scope, handle)?;
        connection
            .query_row(
                "SELECT target, source, event_id, event_json, event_digest, room_id, room_seq,
                        wake_hint_rank,
                        EXISTS(
                            SELECT 1 FROM wake_legacy_events_v2 legacy
                            WHERE legacy.scope = wake_events_v2.scope
                              AND legacy.target = wake_events_v2.target
                              AND legacy.source = wake_events_v2.source
                              AND legacy.event_id = wake_events_v2.event_id
                        )
                 FROM wake_events_v2 WHERE scope = ?1 AND target = ?2 AND room_seq = ?3",
                params![handle.state_id, handle.alias, room_seq],
                |row| {
                    Ok(DeliveredEvent {
                        target: row.get(0)?,
                        source: row.get(1)?,
                        event_id: row.get(2)?,
                        event_json: row.get(3)?,
                        event_digest: row.get(4)?,
                        room_id: row.get(5)?,
                        room_seq: row.get(6)?,
                        wake_hint_rank: row.get(7)?,
                        legacy_metadata: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn pending_seqs_through(
        &self,
        handle: &TargetHandle,
        after_exclusive: i64,
        through_inclusive: i64,
    ) -> Result<Vec<i64>, StoreError> {
        if after_exclusive < 0 || through_inclusive < after_exclusive {
            return Err(StoreError::InvalidPendingRange {
                after_exclusive,
                through_inclusive,
            });
        }
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        assert_current(&connection, &self.scope, handle)?;
        let mut statement = connection.prepare(
            "SELECT room_seq FROM wake_events_v2
             WHERE scope = ?1 AND target = ?2
               AND room_seq > ?3 AND room_seq <= ?4
             ORDER BY room_seq ASC",
        )?;
        let rows = statement.query_map(
            params![
                handle.state_id,
                handle.alias,
                after_exclusive,
                through_inclusive
            ],
            |row| row.get(0),
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn initialize_relay_cursor(
        &self,
        handle: &TargetHandle,
        initial_cursor: i64,
    ) -> Result<i64, StoreError> {
        if initial_cursor < 0 {
            return Err(StoreError::InvalidRelayCursor(initial_cursor));
        }
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_current(&tx, &self.scope, handle)?;
        tx.execute(
            "INSERT OR IGNORE INTO wake_relay_state_v2(scope, target, room_id, cursor)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                handle.state_id,
                handle.alias,
                handle.room_id,
                initial_cursor
            ],
        )?;
        let (stored_room, cursor): (String, i64) = tx.query_row(
            "SELECT room_id, cursor FROM wake_relay_state_v2 WHERE scope = ?1 AND target = ?2",
            params![handle.state_id, handle.alias],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if stored_room != handle.room_id {
            return Err(StoreError::RelayRoomMismatch {
                target: handle.alias.clone(),
                stored_room,
                configured_room: handle.room_id.clone(),
            });
        }
        tx.commit()?;
        Ok(cursor)
    }

    pub fn relay_cursor(&self, handle: &TargetHandle) -> Result<Option<i64>, StoreError> {
        let connection = self.connection.lock().expect("wake store mutex poisoned");
        assert_current(&connection, &self.scope, handle)?;
        let stored = connection
            .query_row(
                "SELECT room_id, cursor FROM wake_relay_state_v2 WHERE scope = ?1 AND target = ?2",
                params![handle.state_id, handle.alias],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        match stored {
            Some((stored_room, _)) if stored_room != handle.room_id => {
                Err(StoreError::RelayRoomMismatch {
                    target: handle.alias.clone(),
                    stored_room,
                    configured_room: handle.room_id.clone(),
                })
            }
            Some((_, cursor)) => Ok(Some(cursor)),
            None => Ok(None),
        }
    }

    pub fn advance_relay_cursor(
        &self,
        handle: &TargetHandle,
        cursor: i64,
    ) -> Result<(), StoreError> {
        if cursor < 0 {
            return Err(StoreError::InvalidRelayCursor(cursor));
        }
        let mut connection = self.connection.lock().expect("wake store mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_current(&tx, &self.scope, handle)?;
        let changed = tx.execute(
            "UPDATE wake_relay_state_v2 SET cursor = MAX(cursor, ?4)
             WHERE scope = ?1 AND target = ?2 AND room_id = ?3",
            params![handle.state_id, handle.alias, handle.room_id, cursor],
        )?;
        if changed != 1 {
            return Err(StoreError::MissingRelayState(handle.alias.clone()));
        }
        tx.commit()?;
        Ok(())
    }

    fn load_ack_state(
        &self,
        connection: &Connection,
        handle: &TargetHandle,
    ) -> Result<AckState, StoreError> {
        let (last_acked_seq, max_read_seq) = connection
            .query_row(
                "SELECT last_acked_seq, max_read_seq FROM wake_target_state_v2
             WHERE scope = ?1 AND target = ?2",
                params![handle.state_id, handle.alias],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::MissingTargetState(handle.alias.clone()))?;
        let max_pending_seq = connection.query_row(
            "SELECT MAX(room_seq) FROM wake_events_v2
             WHERE scope = ?1 AND target = ?2 AND room_seq > ?3",
            params![handle.state_id, handle.alias, last_acked_seq],
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
        handle: &TargetHandle,
    ) -> Result<i64, StoreError> {
        connection
            .query_row(
                "SELECT last_acked_seq FROM wake_target_state_v2 WHERE scope = ?1 AND target = ?2",
                params![handle.state_id, handle.alias],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::MissingTargetState(handle.alias.clone()))
    }
}

fn validate_scope(scope: &str) -> Result<(), StoreError> {
    if scope.trim().is_empty() {
        Err(StoreError::InvalidScope)
    } else {
        Ok(())
    }
}

fn validate_target(
    identity: &str,
    alias: &str,
    room_id: &str,
    room_tip: i64,
) -> Result<(), StoreError> {
    if identity.trim().is_empty() {
        return Err(StoreError::InvalidTargetIdentity);
    }
    if alias.trim().is_empty() {
        return Err(StoreError::InvalidTargetAlias);
    }
    if room_id.trim().is_empty() {
        return Err(StoreError::InvalidTargetRoom);
    }
    if room_tip < 0 {
        return Err(StoreError::InvalidRoomTip(room_tip));
    }
    Ok(())
}

fn load_binding(
    connection: &Connection,
    scope: &str,
    alias: &str,
) -> Result<Option<(String, String, String, i64)>, StoreError> {
    connection
        .query_row(
            "SELECT target_identity, room_id, state_id, floor_seq
             FROM wake_target_bindings_v2
             WHERE scope = ?1 AND target_alias = ?2",
            params![scope, alias],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(StoreError::from)
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, StoreError> {
    Ok(connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
         )",
        [table],
        |row| row.get(0),
    )?)
}

fn legacy_targets(connection: &Connection) -> Result<Vec<String>, StoreError> {
    let mut targets = BTreeSet::new();
    for table in ["wake_events", "wake_target_state"] {
        if !table_exists(connection, table)? {
            continue;
        }
        let mut statement = connection.prepare(&format!("SELECT DISTINCT target FROM {table}"))?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        for target in rows {
            targets.insert(target?);
        }
    }
    Ok(targets.into_iter().collect())
}

fn delete_legacy_target(connection: &Connection, alias: &str) -> Result<(), StoreError> {
    for table in ["wake_events", "wake_target_state"] {
        if table_exists(connection, table)? {
            connection.execute(&format!("DELETE FROM {table} WHERE target = ?1"), [alias])?;
        }
    }
    Ok(())
}

fn rotate_target_state(
    connection: &Connection,
    scope: &str,
    identity: &str,
    alias: &str,
    room_id: &str,
    room_tip: i64,
    old_state_id: Option<&str>,
) -> Result<TargetHandle, StoreError> {
    let state_id = uuid::Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO wake_target_state_v2
             (scope, target, last_acked_seq, max_read_seq)
         VALUES (?1, ?2, ?3, ?3)",
        params![state_id, alias, room_tip],
    )?;
    connection.execute(
        "INSERT INTO wake_relay_state_v2(scope, target, room_id, cursor)
         VALUES (?1, ?2, ?3, ?4)",
        params![state_id, alias, room_id, room_tip],
    )?;
    connection.execute(
        "INSERT INTO wake_target_bindings_v2
             (scope, target_alias, target_identity, room_id, state_id, floor_seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(scope, target_alias) DO UPDATE SET
             target_identity = excluded.target_identity,
             room_id = excluded.room_id,
             state_id = excluded.state_id,
             floor_seq = excluded.floor_seq",
        params![scope, alias, identity, room_id, state_id, room_tip],
    )?;
    if let Some(old_state_id) = old_state_id {
        for table in [
            "wake_read_cursors_v2",
            "wake_legacy_events_v2",
            "wake_events_v2",
            "wake_target_state_v2",
            "wake_relay_state_v2",
        ] {
            connection.execute(
                &format!("DELETE FROM {table} WHERE scope = ?1 AND target = ?2"),
                params![old_state_id, alias],
            )?;
        }
    }
    Ok(TargetHandle {
        identity: identity.to_string(),
        state_id,
        alias: alias.to_string(),
        room_id: room_id.to_string(),
        floor_seq: room_tip,
    })
}

fn assert_current(
    connection: &Connection,
    scope: &str,
    handle: &TargetHandle,
) -> Result<(), StoreError> {
    let matches = connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM wake_target_bindings_v2
             WHERE scope = ?1 AND target_alias = ?2 AND target_identity = ?3
               AND state_id = ?4 AND room_id = ?5 AND floor_seq = ?6
         )",
        params![
            scope,
            handle.alias,
            handle.identity,
            handle.state_id,
            handle.room_id,
            handle.floor_seq
        ],
        |row| row.get::<_, bool>(0),
    )?;
    if matches {
        Ok(())
    } else {
        Err(StoreError::StaleTargetState {
            alias: handle.alias.clone(),
            identity: handle.identity.clone(),
            state_id: handle.state_id.clone(),
        })
    }
}

fn validate_returned_cursors(returned: &[i64]) -> Result<(), StoreError> {
    let mut previous = None;
    for cursor in returned {
        if *cursor < 0 {
            return Err(StoreError::InvalidReadCursor(*cursor));
        }
        if previous.is_some_and(|previous| previous >= *cursor) {
            return Err(StoreError::NonMonotonicReadCursor(*cursor));
        }
        previous = Some(*cursor);
    }
    Ok(())
}

fn ensure_pending_range_read(
    connection: &Connection,
    handle: &TargetHandle,
    after_exclusive: i64,
    through_inclusive: i64,
) -> Result<(), StoreError> {
    let missing: Option<i64> = connection
        .query_row(
            "SELECT e.room_seq FROM wake_events_v2 e
             WHERE e.scope = ?1 AND e.target = ?2
               AND e.room_seq > ?3 AND e.room_seq <= ?4
               AND NOT EXISTS (
                   SELECT 1 FROM wake_read_cursors_v2 r
                   WHERE r.scope = e.scope AND r.target = e.target
                     AND r.cursor = e.room_seq
               )
             ORDER BY e.room_seq ASC LIMIT 1",
            params![
                handle.state_id,
                handle.alias,
                after_exclusive,
                through_inclusive
            ],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(missing_seq) = missing {
        Err(StoreError::UnreadPendingEvent {
            cursor: through_inclusive,
            missing_seq,
        })
    } else {
        Ok(())
    }
}

fn lock_directory_for(path: &Path) -> Result<PathBuf, StoreError> {
    let file_name = path
        .file_name()
        .ok_or_else(|| StoreError::InvalidDatabasePath(path.to_path_buf()))?;
    Ok(path.with_file_name(format!("{}.locks", file_name.to_string_lossy())))
}

fn target_lock_path(lock_dir: &Path, scope: &str, alias: &str) -> PathBuf {
    let digest = Sha256::digest(format!("{scope}\0{alias}").as_bytes());
    lock_dir.join(format!("{digest:x}.lock"))
}

fn process_file_lock(path: &Path) -> Arc<AsyncMutex<()>> {
    let locks = PROCESS_FILE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks
        .lock()
        .expect("wake global process-lock map mutex poisoned");
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(AsyncMutex::new(()));
    locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
    lock
}

fn canonical_database_path(path: &Path) -> Result<PathBuf, StoreError> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|source| StoreError::CanonicalizeDatabase(path.to_path_buf(), source))?;
    let metadata = std::fs::metadata(&canonical)
        .map_err(|source| StoreError::InspectDatabase(canonical.clone(), source))?;
    if !metadata.is_file() {
        return Err(StoreError::UnsafeDatabaseFile(canonical));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // A second hard-link name would derive another lock namespace even
        // though SQLite opens the same inode. Reject it rather than weakening
        // the reset fence.
        if metadata.nlink() != 1 {
            return Err(StoreError::UnsafeDatabaseFile(canonical));
        }
    }
    Ok(canonical)
}

fn open_private_lock_file(path: &Path) -> Result<File, StoreError> {
    let open_existing = || {
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        options.open(path)
    };
    let mut create = std::fs::OpenOptions::new();
    create.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        create
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = match create.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => open_existing()
            .map_err(|source| StoreError::OpenTargetLock(path.to_path_buf(), source))?,
        Err(source) => return Err(StoreError::OpenTargetLock(path.to_path_buf(), source)),
    };
    let file_metadata = file
        .metadata()
        .map_err(|source| StoreError::InspectTargetLock(path.to_path_buf(), source))?;
    let path_metadata = std::fs::symlink_metadata(path)
        .map_err(|source| StoreError::InspectTargetLock(path.to_path_buf(), source))?;
    if !file_metadata.is_file()
        || path_metadata.file_type().is_symlink()
        || !path_metadata.is_file()
    {
        return Err(StoreError::UnsafeTargetLockFile(path.to_path_buf()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if file_metadata.dev() != path_metadata.dev()
            || file_metadata.ino() != path_metadata.ino()
            || file_metadata.nlink() != 1
        {
            return Err(StoreError::UnsafeTargetLockFile(path.to_path_buf()));
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(StoreError::SetPermissions)?;
    }
    Ok(file)
}

fn create_private_directory(path: &Path) -> Result<(), StoreError> {
    if path.as_os_str().is_empty() || path.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(StoreError::CreateParent)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(path).map_err(StoreError::CreateParent)?;
    Ok(())
}

fn create_private_file(path: &Path) -> Result<(), StoreError> {
    if path.exists() {
        return Ok(());
    }
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(StoreError::CreateDatabase(error)),
    }
}

fn set_private_file_permissions(path: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(StoreError::SetPermissions)?;
    }
    Ok(())
}

fn set_private_directory_permissions(path: &Path) -> Result<(), StoreError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|source| StoreError::InspectTargetLock(path.to_path_buf(), source))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StoreError::UnsafeTargetLockDirectory(path.to_path_buf()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(StoreError::SetPermissions)?;
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("state scope must be non-empty")]
    InvalidScope,
    #[error("target identity must be non-empty")]
    InvalidTargetIdentity,
    #[error("target alias must be non-empty")]
    InvalidTargetAlias,
    #[error("target room must be non-empty")]
    InvalidTargetRoom,
    #[error("room tip must be non-negative, got {0}")]
    InvalidRoomTip(i64),
    #[error("invalid wake database path {0}")]
    InvalidDatabasePath(PathBuf),
    #[error("failed to canonicalize wake database {0}: {1}")]
    CanonicalizeDatabase(PathBuf, #[source] std::io::Error),
    #[error("failed to inspect wake database {0}: {1}")]
    InspectDatabase(PathBuf, #[source] std::io::Error),
    #[error("wake database must be a uniquely named regular file: {0}")]
    UnsafeDatabaseFile(PathBuf),
    #[error("failed to create wake database directory: {0}")]
    CreateParent(#[source] std::io::Error),
    #[error("failed to create wake database: {0}")]
    CreateDatabase(#[source] std::io::Error),
    #[error("failed to harden wake database permissions: {0}")]
    SetPermissions(#[source] std::io::Error),
    #[error("failed to open target lock {0}: {1}")]
    OpenTargetLock(PathBuf, #[source] std::io::Error),
    #[error("failed to inspect target lock path {0}: {1}")]
    InspectTargetLock(PathBuf, #[source] std::io::Error),
    #[error("target lock directory is not a real directory: {0}")]
    UnsafeTargetLockDirectory(PathBuf),
    #[error("target lock path is not a regular file: {0}")]
    UnsafeTargetLockFile(PathBuf),
    #[error("failed to acquire target lock {0}: {1}")]
    AcquireTargetLock(PathBuf, #[source] std::io::Error),
    #[error("target-lock worker failed: {0}")]
    TargetLockTask(#[source] tokio::task::JoinError),
    #[error("wake database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "legacy v0.6 wake state is present for targets {0:?}; run `cowchat-codex migrate-legacy-state --target <alias>` for each target, or explicitly discard one with `reset-state --target <alias> --discard-legacy-state`"
    )]
    LegacyStatePresent(Vec<String>),
    #[error("legacy wake target {0:?} was not found")]
    LegacyTargetNotFound(String),
    #[error("target {0:?} already has current-generation state")]
    TargetAlreadyInitialized(String),
    #[error("legacy wake target {0:?} contains invalid cursor state")]
    InvalidLegacyCursor(String),
    #[error("legacy target {alias:?} contains room {stored_room:?}, not configured room {configured_room:?}")]
    LegacyTargetRoomMismatch {
        alias: String,
        stored_room: String,
        configured_room: String,
    },
    #[error("target {alias:?} is bound to room {stored_room:?}, not {configured_room:?}")]
    TargetBindingRoomMismatch {
        alias: String,
        stored_room: String,
        configured_room: String,
    },
    #[error("target state for {0:?} is missing")]
    MissingTargetState(String),
    #[error("target handle is stale: alias={alias:?} identity={identity:?} state={state_id:?}")]
    StaleTargetState {
        alias: String,
        identity: String,
        state_id: String,
    },
    #[error("room tip {room_tip} is behind retained target state (floor {floor_seq}, acknowledged {last_acked_seq}, delivered {max_delivered_seq})")]
    RoomTipBehindCursor {
        room_tip: i64,
        floor_seq: i64,
        last_acked_seq: i64,
        max_delivered_seq: i64,
    },
    #[error("event room {event_room:?} does not match configured room {configured_room:?}")]
    EventRoomMismatch {
        configured_room: String,
        event_room: String,
    },
    #[error("event id was reused with different content: target={target} source={event_source} id={event_id}")]
    IdempotencyConflict {
        target: String,
        event_source: String,
        event_id: String,
    },
    #[error("wake event reservation disappeared")]
    MissingReservation,
    #[error(
        "delivery is already owned by another live bridge process; retry after its lease expires"
    )]
    DeliveryInProgress,
    #[error("delivery claim generation overflow")]
    GenerationOverflow,
    #[error("lost a delivery claim race; retry the event")]
    LostClaimRace,
    #[error("delivery claim is stale or the reservation was already completed")]
    StaleDeliveryClaim,
    #[error("lease seconds must be positive, got {0}")]
    InvalidLeaseSeconds(i64),
    #[error("delivered sequence must be non-negative, got {0}")]
    InvalidDeliveredSequence(i64),
    #[error("cannot record delivered sequence {room_seq} at or behind acknowledged cursor {last_acked_seq}")]
    DeliveryBehindCursor { room_seq: i64, last_acked_seq: i64 },
    #[error("read cursor {cursor} was not acknowledged or returned previously (last acknowledged {last_acked})")]
    UnauthorizedReadCursor { cursor: i64, last_acked: i64 },
    #[error("returned read cursor must be non-negative, got {0}")]
    InvalidReadCursor(i64),
    #[error("returned read cursors must be strictly increasing; got {0}")]
    NonMonotonicReadCursor(i64),
    #[error("cannot grant read authority for sequence {0}; it is not a locally delivered wake")]
    ReadCursorNotDelivered(i64),
    #[error(
        "cannot grant cursor {cursor}; pending event {missing_seq} was not returned by a read"
    )]
    UnreadPendingEvent { cursor: i64, missing_seq: i64 },
    #[error("invalid pending range ({after_exclusive}, {through_inclusive}]")]
    InvalidPendingRange {
        after_exclusive: i64,
        through_inclusive: i64,
    },
    #[error(
        "cannot acknowledge cursor {cursor}; highest cursor returned by read is {max_read_seq}"
    )]
    AckBeyondRead { cursor: i64, max_read_seq: i64 },
    #[error("cannot move acknowledgement backwards from {last_acked_seq} to {cursor}")]
    AckRegressed { cursor: i64, last_acked_seq: i64 },
    #[error("cannot acknowledge cursor {0}; that exact cursor was not returned by a read")]
    AckCursorNotReturned(i64),
    #[error("relay cursor must be non-negative, got {0}")]
    InvalidRelayCursor(i64),
    #[error("relay target {target:?} is bound to room {stored_room:?}, not configured room {configured_room:?}")]
    RelayRoomMismatch {
        target: String,
        stored_room: String,
        configured_room: String,
    },
    #[error("relay state for target {0:?} was not initialized")]
    MissingRelayState(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reservation<'a>(event_id: &'a str, event_json: &'a str) -> EventReservation<'a> {
        EventReservation {
            source: "ci",
            event_id,
            request_json: event_json,
            event_json,
            event_digest: "digest",
            room_id: "room",
            wake_hint_rank: 1,
            now_unix: 1,
        }
    }

    fn activate(store: &WakeStore) -> TargetHandle {
        store
            .activate_target("identity-a", "reviewer", "room", 0)
            .unwrap()
    }

    fn deliver(store: &WakeStore, handle: &TargetHandle, event_id: &str, seq: i64) {
        store
            .reserve_event(handle, reservation(event_id, event_id))
            .unwrap();
        let DeliveryClaim::Claimed { generation } =
            store.claim_delivery(handle, "ci", event_id, 1, 30).unwrap()
        else {
            panic!("new reservation must be claimable")
        };
        store
            .mark_delivered(handle, "ci", event_id, generation, seq, event_id)
            .unwrap();
    }

    #[test]
    fn delivery_claim_is_cross_process_leased_renewed_and_generation_fenced() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("wake.db");
        let first_process = WakeStore::open(&path, "scope").unwrap();
        let second_process = WakeStore::open(&path, "scope").unwrap();
        let first_handle = activate(&first_process);
        let second_handle = activate(&second_process);
        assert_eq!(first_handle, second_handle);
        let first = first_process
            .reserve_event(&first_handle, reservation("evt-1", "event-time-1"))
            .unwrap();
        let retry = second_process
            .reserve_event(&second_handle, reservation("evt-1", "event-time-1"))
            .unwrap();
        assert!(!first.duplicate);
        assert!(retry.duplicate);
        assert_eq!(retry.event_json, "event-time-1");
        assert_eq!(
            first_process
                .claim_delivery(&first_handle, "ci", "evt-1", 10, 30)
                .unwrap(),
            DeliveryClaim::Claimed { generation: 1 }
        );
        assert_eq!(
            second_process
                .claim_delivery(&second_handle, "ci", "evt-1", 11, 30)
                .unwrap(),
            DeliveryClaim::InFlight
        );
        assert!(first_process
            .renew_delivery_claim(&first_handle, "ci", "evt-1", 1, 35)
            .unwrap());
        assert_eq!(
            second_process
                .claim_delivery(&second_handle, "ci", "evt-1", 60, 30)
                .unwrap(),
            DeliveryClaim::InFlight
        );
        assert_eq!(
            second_process
                .claim_delivery(&second_handle, "ci", "evt-1", 66, 30)
                .unwrap(),
            DeliveryClaim::Claimed { generation: 2 }
        );
        assert!(!first_process
            .renew_delivery_claim(&first_handle, "ci", "evt-1", 1, 67)
            .unwrap());
        assert!(!first_process
            .release_delivery(&first_handle, "ci", "evt-1", 1)
            .unwrap());
        second_process
            .mark_delivered(&second_handle, "ci", "evt-1", 2, 7, "msg-7")
            .unwrap();
        assert_eq!(
            first_process
                .claim_delivery(&first_handle, "ci", "evt-1", 68, 30)
                .unwrap(),
            DeliveryClaim::Delivered(7)
        );
    }

    #[test]
    fn read_and_ack_require_every_pending_event_through_cursor() {
        let store = WakeStore::open_in_memory().unwrap();
        let handle = activate(&store);
        deliver(&store, &handle, "evt-5", 5);
        deliver(&store, &handle, "evt-10", 10);
        assert_eq!(
            store.pending_seqs_through(&handle, 0, 10).unwrap(),
            vec![5, 10]
        );
        assert!(matches!(
            store.record_read(&handle, &[10]),
            Err(StoreError::UnreadPendingEvent {
                cursor: 10,
                missing_seq: 5
            })
        ));
        assert!(store.authorize_read_cursor(&handle, 10).is_err());
        store.record_read(&handle, &[5]).unwrap();
        store.record_read(&handle, &[10]).unwrap();
        assert_eq!(store.acknowledge(&handle, 10).unwrap().last_acked_seq, 10);
    }

    #[test]
    fn target_identity_change_and_reset_fence_stale_handles_without_touching_peer() {
        let store = WakeStore::open_in_memory().unwrap();
        let reviewer = activate(&store);
        let peer = store
            .activate_target("identity-b", "builder", "room-b", 4)
            .unwrap();
        let changed = store
            .activate_target("identity-a2", "reviewer", "room-2", 8)
            .unwrap();
        assert_ne!(reviewer.state_id, changed.state_id);
        assert!(matches!(
            store.last_acked_seq(&reviewer),
            Err(StoreError::StaleTargetState { .. })
        ));
        assert_eq!(store.last_acked_seq(&peer).unwrap(), 4);
        let reset = store
            .reset_target("identity-a2", "reviewer", "room-2", 12)
            .unwrap();
        assert_ne!(changed.state_id, reset.state_id);
        assert_eq!(store.last_acked_seq(&reset).unwrap(), 12);
        assert_eq!(store.relay_cursor(&reset).unwrap(), Some(12));
        assert!(matches!(
            store.last_acked_seq(&changed),
            Err(StoreError::StaleTargetState { .. })
        ));
    }

    #[test]
    fn activation_fails_closed_on_floor_ack_or_delivered_rollback() {
        let store = WakeStore::open_in_memory().unwrap();
        let handle = store
            .activate_target("identity-a", "reviewer", "room", 10)
            .unwrap();
        assert!(matches!(
            store.activate_target("identity-a", "reviewer", "room", 9),
            Err(StoreError::RoomTipBehindCursor { .. })
        ));
        store
            .reserve_event(&handle, reservation("evt-15", "event"))
            .unwrap();
        let DeliveryClaim::Claimed { generation } = store
            .claim_delivery(&handle, "ci", "evt-15", 1, 30)
            .unwrap()
        else {
            panic!()
        };
        assert!(matches!(
            store.mark_delivered(&handle, "ci", "evt-15", generation, 10, "msg"),
            Err(StoreError::DeliveryBehindCursor { .. })
        ));
        store
            .mark_delivered(&handle, "ci", "evt-15", generation, 15, "msg")
            .unwrap();
        assert!(matches!(
            store.activate_target("identity-a", "reviewer", "room", 14),
            Err(StoreError::RoomTipBehindCursor {
                max_delivered_seq: 15,
                ..
            })
        ));
        let reset = store
            .reset_target("identity-a", "reviewer", "room", 3)
            .unwrap();
        assert_eq!(reset.floor_seq, 3);
    }

    #[cfg(unix)]
    #[test]
    fn database_sidecars_and_target_lock_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("private").join("wake.db");
        let store = WakeStore::open(&path, "scope").unwrap();
        let _guard = store.lock_target_shared("reviewer").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        for sidecar in [
            PathBuf::from(format!("{}-wal", path.display())),
            PathBuf::from(format!("{}-shm", path.display())),
        ] {
            if sidecar.exists() {
                assert_eq!(
                    std::fs::metadata(sidecar).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
        let lock_dir = lock_directory_for(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&lock_dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for entry in std::fs::read_dir(lock_dir).unwrap() {
            assert_eq!(
                entry.unwrap().metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn target_lock_excludes_a_concurrent_reset_owner() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("wake.db");
        let store = WakeStore::open(&path, "scope").unwrap();
        let shared = store.lock_target_shared("reviewer").unwrap();
        let digest = Sha256::digest(b"scope\0reviewer");
        let lock_path = lock_directory_for(&path)
            .unwrap()
            .join(format!("{digest:x}.lock"));
        let contender = open_private_lock_file(&lock_path).unwrap();
        assert!(FileExt::try_lock_exclusive(&contender).is_err());
        drop(shared);
        FileExt::try_lock_exclusive(&contender).unwrap();
        FileExt::unlock(&contender).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn database_symlink_alias_uses_the_canonical_lock_namespace() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("wake.db");
        let alias = temp.path().join("wake-alias.db");
        let real_store = WakeStore::open(&real, "scope").unwrap();
        symlink(&real, &alias).unwrap();
        let alias_store = WakeStore::open(&alias, "scope").unwrap();
        assert_eq!(real_store.lock_dir, alias_store.lock_dir);

        let held = real_store.lock_target_shared("reviewer").unwrap();
        let lock_path =
            target_lock_path(alias_store.lock_dir.as_ref().unwrap(), "scope", "reviewer");
        let contender = open_private_lock_file(&lock_path).unwrap();
        assert!(FileExt::try_lock_exclusive(&contender).is_err());
        drop(held);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn database_symlink_alias_cannot_bypass_a_live_reset_fence() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("wake.db");
        let alias = temp.path().join("wake-alias.db");
        let real_store = Arc::new(WakeStore::open(&real, "scope").unwrap());
        let old = real_store
            .activate_target("identity", "reviewer", "room", 7)
            .unwrap();
        symlink(&real, &alias).unwrap();
        let alias_store = Arc::new(WakeStore::open(&alias, "scope").unwrap());
        let held = real_store
            .lock_target_exclusive_async("reviewer")
            .await
            .unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let reset = {
            let store = alias_store.clone();
            tokio::spawn(async move {
                started_tx.send(()).unwrap();
                let _guard = store.lock_target_exclusive_async("reviewer").await.unwrap();
                store
                    .reset_target("identity", "reviewer", "room", 8)
                    .unwrap()
            })
        };
        started_rx.await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(!reset.is_finished());
        drop(held);
        let rotated = tokio::time::timeout(std::time::Duration::from_secs(1), reset)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(rotated.state_id, old.state_id);
        assert!(matches!(
            real_store.last_acked_seq(&old),
            Err(StoreError::StaleTargetState { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn lockfile_symlink_swap_fails_closed_without_touching_victim() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("wake.db");
        let store = WakeStore::open(&path, "scope").unwrap();
        let victim = temp.path().join("victim");
        std::fs::write(&victim, b"unchanged").unwrap();
        let lock_path = target_lock_path(store.lock_dir.as_ref().unwrap(), "scope", "reviewer");
        symlink(&victim, &lock_path).unwrap();
        assert!(store.lock_target_exclusive("reviewer").is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"unchanged");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn async_target_lock_wait_does_not_starve_the_runtime_worker() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("wake.db");
        let holder_store = WakeStore::open(&path, "scope").unwrap();
        let waiter_store = Arc::new(WakeStore::open(&path, "scope").unwrap());
        let held = holder_store.lock_target_exclusive("reviewer").unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            drop(held);
        });

        let waiter = {
            let store = waiter_store.clone();
            tokio::spawn(async move { store.lock_target_exclusive_async("reviewer").await })
        };
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            tokio::time::sleep(std::time::Duration::from_millis(10)),
        )
        .await
        .expect("a waiting filesystem lock must not park the only Tokio worker");
        release.join().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[test]
    fn legacy_state_fails_closed_then_migrates_delivered_unacked_event() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("wake.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE wake_events (
                     target TEXT NOT NULL, source TEXT NOT NULL, event_id TEXT NOT NULL,
                     event_json TEXT NOT NULL, room_id TEXT NOT NULL,
                     wake_hint_rank INTEGER NOT NULL, room_seq INTEGER, message_id TEXT,
                     created_at INTEGER NOT NULL,
                     PRIMARY KEY (target, source, event_id)
                 );
                 CREATE TABLE wake_target_state (
                     target TEXT PRIMARY KEY, last_acked_seq INTEGER NOT NULL DEFAULT 0,
                     max_read_seq INTEGER NOT NULL DEFAULT 0, wake_claimed_at INTEGER
                 );",
            )
            .unwrap();
        let event_json = serde_json::json!({
            "specversion": "1.0",
            "id": "legacy-event",
            "source": "ci",
            "type": "review.ready",
            "time": "2026-01-01T00:00:00Z",
            "data": {"commit": "abc"}
        })
        .to_string();
        connection
            .execute(
                "INSERT INTO wake_target_state(target, last_acked_seq, max_read_seq)
                 VALUES ('reviewer', 6, 7)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO wake_events
                     (target, source, event_id, event_json, room_id, wake_hint_rank,
                      room_seq, message_id, created_at)
                 VALUES ('reviewer', 'ci', 'legacy-event', ?1, 'room', 1, 7, 'msg-7', 1)",
                [&event_json],
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            WakeStore::open(&path, "scope"),
            Err(StoreError::LegacyStatePresent(targets)) if targets == vec!["reviewer"]
        ));

        let store = WakeStore::open_for_legacy_maintenance(&path, "scope").unwrap();
        let handle = store
            .migrate_legacy_target("identity", "reviewer", "room", 9)
            .unwrap();
        assert_eq!(handle.floor_seq, 6);
        assert_eq!(store.last_acked_seq(&handle).unwrap(), 6);
        let migrated = store.delivered_event(&handle, 7).unwrap().unwrap();
        assert!(migrated.legacy_metadata);
        assert_eq!(migrated.event_json, event_json);

        store.authorize_read_cursor(&handle, 6).unwrap();
        let read = store.record_read(&handle, &[7]).unwrap();
        assert_eq!(read.max_read_seq, 7);
        let acknowledged = store.acknowledge(&handle, 7).unwrap();
        assert_eq!(acknowledged.last_acked_seq, 7);
        drop(store);

        let reopened = WakeStore::open(&path, "scope").unwrap();
        assert_eq!(
            reopened
                .current_target("identity", "reviewer", "room")
                .unwrap()
                .unwrap()
                .state_id,
            handle.state_id
        );
    }

    #[test]
    fn production_store_rejects_non_durable_sqlite_memory_uris() {
        for path in [":memory:", "file:wake?mode=memory&cache=shared"] {
            assert!(matches!(
                WakeStore::open(Path::new(path), "scope"),
                Err(StoreError::InvalidDatabasePath(_))
            ));
        }
    }
}
