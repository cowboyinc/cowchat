use chrono::{DateTime, Utc};
use cowchat_core::{
    ChatMessage, HandoffAcceptancePacket, HandoffPacket, Room, HANDOFF_ACCEPTED_KIND,
    HANDOFF_READY_KIND, HANDOFF_SCHEMA_VERSION,
};
use dashmap::DashMap;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use crate::connection::{matches_room_owner, LEGACY_UNOWNED_OWNER_KEY};

pub struct Store {
    conn: Mutex<Connection>,
    /// In-memory mirror of `room_grants` (api_key -> granted room_ids).
    /// Consulted on the per-broadcast access hot path so grant checks never
    /// touch SQLite. Hydrated at open, updated on redemption/room destroy.
    grants: DashMap<String, HashSet<String>>,
}

pub(crate) const MAX_ROOM_NAME_CHARS: usize = 100;

/// Canonicalize and validate a room name at the server boundary.
///
/// Names are identifiers in CLI resolution and SQLite's uniqueness constraint,
/// so every creation and rename must use the same canonical form.
pub(crate) fn normalize_room_name(name: &str) -> Result<String, String> {
    let normalized = name.trim();
    if normalized.is_empty() {
        return Err("Room name cannot be empty".to_string());
    }
    if normalized.chars().any(char::is_control) {
        return Err("Room name cannot contain control characters".to_string());
    }
    if normalized.chars().count() > MAX_ROOM_NAME_CHARS {
        return Err(format!(
            "Room name cannot exceed {MAX_ROOM_NAME_CHARS} Unicode scalar values"
        ));
    }
    Ok(normalized.to_string())
}

#[derive(Debug, Clone)]
pub struct InviteMeta {
    pub room_id: String,
    pub created_by_key: String,
    pub single_use: bool,
    pub redeemed_count: i64,
    pub revoked: bool,
}

#[derive(Debug, Clone)]
pub struct VoteMeta {
    pub vote_id: String,
    pub room_id: String,
    pub title: String,
    pub description: Option<String>,
    pub options: Vec<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub closes_at: Option<DateTime<Utc>>,
    pub status: String,
    pub eligible_voters: usize,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        crate::auth::harden_parent_directory(path)
            .map_err(|_| rusqlite::Error::InvalidPath(path.to_path_buf()))?;
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        drop(
            options
                .open(path)
                .map_err(|_| rusqlite::Error::InvalidPath(path.to_path_buf()))?,
        );
        crate::auth::harden_file_permissions(path)
            .map_err(|_| rusqlite::Error::InvalidPath(path.to_path_buf()))?;
        let conn = Connection::open(path)?;
        let store = Self {
            conn: Mutex::new(conn),
            grants: DashMap::new(),
        };
        store.initialize()?;
        for candidate in [
            path.to_path_buf(),
            std::path::PathBuf::from(format!("{}-wal", path.display())),
            std::path::PathBuf::from(format!("{}-shm", path.display())),
        ] {
            if candidate.exists() {
                crate::auth::harden_file_permissions(&candidate)
                    .map_err(|_| rusqlite::Error::InvalidPath(candidate))?;
            }
        }
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Mutex::new(conn),
            grants: DashMap::new(),
        };
        store.initialize()?;
        Ok(store)
    }

    fn initialize(&self) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();

        // Step 1: Create tables (without new columns — old DBs may already have rooms table)
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS api_keys (
                api_key    TEXT PRIMARY KEY,
                tier       TEXT NOT NULL DEFAULT 'free',
                label      TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );

            CREATE TABLE IF NOT EXISTS agent_identities (
                agent_id  TEXT PRIMARY KEY,
                owner_key TEXT NOT NULL,
                claimed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );

            CREATE TABLE IF NOT EXISTS rooms (
                room_id     TEXT PRIMARY KEY,
                name        TEXT NOT NULL UNIQUE,
                description TEXT,
                parent_id   TEXT REFERENCES rooms(room_id) ON DELETE SET NULL,
                created_by  TEXT,
                created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                visibility  TEXT NOT NULL DEFAULT 'private',
                owner_key   TEXT,
                encrypted   INTEGER NOT NULL DEFAULT 0,
                CHECK (room_id != parent_id)
            );

            CREATE TABLE IF NOT EXISTS messages (
                message_id       TEXT PRIMARY KEY,
                room_id          TEXT NOT NULL REFERENCES rooms(room_id) ON DELETE CASCADE,
                agent_id         TEXT NOT NULL,
                agent_name       TEXT NOT NULL,
                content          TEXT NOT NULL,
                reply_to_message TEXT REFERENCES messages(message_id) ON DELETE SET NULL,
                metadata         TEXT DEFAULT '{}',
                created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                seq              INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_messages_room_time
                ON messages(room_id, created_at DESC);

            CREATE INDEX IF NOT EXISTS idx_messages_reply
                ON messages(reply_to_message) WHERE reply_to_message IS NOT NULL;
            -- idx_messages_room_seq is created in Step 2, after the seq migration runs,
            -- because old DBs may not have the seq column at this point.

            CREATE TABLE IF NOT EXISTS room_sequences (
                room_id    TEXT PRIMARY KEY REFERENCES rooms(room_id) ON DELETE CASCADE,
                high_water INTEGER NOT NULL DEFAULT 0 CHECK (high_water >= 0)
            );

            CREATE TABLE IF NOT EXISTS agent_sessions (
                session_id      TEXT PRIMARY KEY,
                agent_id        TEXT NOT NULL,
                agent_name      TEXT NOT NULL,
                capabilities    TEXT DEFAULT '[]',
                connected_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                disconnected_at TEXT
            );

            CREATE TABLE IF NOT EXISTS votes (
                vote_id         TEXT PRIMARY KEY,
                room_id         TEXT NOT NULL,
                title           TEXT NOT NULL,
                description     TEXT,
                options         TEXT NOT NULL DEFAULT '[]',
                created_by      TEXT NOT NULL,
                created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                closes_at       TEXT,
                status          TEXT NOT NULL DEFAULT 'open',
                eligible_voters INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS vote_ballots (
                vote_id      TEXT NOT NULL REFERENCES votes(vote_id) ON DELETE CASCADE,
                agent_id     TEXT NOT NULL,
                agent_name   TEXT NOT NULL,
                option_index INTEGER NOT NULL,
                cast_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                PRIMARY KEY (vote_id, agent_id)
            );

            CREATE TABLE IF NOT EXISTS vote_eligible_agents (
                vote_id  TEXT NOT NULL REFERENCES votes(vote_id) ON DELETE CASCADE,
                agent_id TEXT NOT NULL,
                PRIMARY KEY (vote_id, agent_id)
            );

            CREATE TABLE IF NOT EXISTS room_tasks (
                task_id     TEXT PRIMARY KEY,
                room_id     TEXT NOT NULL REFERENCES rooms(room_id) ON DELETE CASCADE,
                title       TEXT NOT NULL,
                description TEXT,
                status      TEXT NOT NULL,
                assignee    TEXT,
                created_by  TEXT NOT NULL,
                created_at  TEXT NOT NULL,
                updated_at  TEXT,
                note        TEXT
            );

            CREATE TABLE IF NOT EXISTS subscriptions (
                subscription_id     TEXT PRIMARY KEY,
                room_id             TEXT NOT NULL,
                owner_key           TEXT NOT NULL,
                webhook_url         TEXT NOT NULL,
                secret              TEXT NOT NULL,
                kinds               TEXT,         -- JSON array; NULL = match all kinds
                only_from           TEXT,
                not_from            TEXT,
                exclude_thinking    INTEGER NOT NULL DEFAULT 0,
                since_seq           INTEGER NOT NULL DEFAULT 0,
                last_delivered_seq  INTEGER NOT NULL DEFAULT 0,
                status              TEXT NOT NULL DEFAULT 'active',  -- active | failed | disabled
                failure_count       INTEGER NOT NULL DEFAULT 0,
                created_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_subscriptions_room ON subscriptions(room_id);
            CREATE INDEX IF NOT EXISTS idx_subscriptions_status ON subscriptions(status);

            CREATE TABLE IF NOT EXISTS subscription_deliveries (
                delivery_id      TEXT PRIMARY KEY,
                subscription_id  TEXT NOT NULL REFERENCES subscriptions(subscription_id) ON DELETE CASCADE,
                message_seq      INTEGER NOT NULL,
                message_id       TEXT NOT NULL,
                next_attempt_at  TEXT NOT NULL,
                attempts         INTEGER NOT NULL DEFAULT 0,
                last_error       TEXT,
                UNIQUE(subscription_id, message_seq)
            );
            CREATE INDEX IF NOT EXISTS idx_deliveries_pending ON subscription_deliveries(next_attempt_at);

            CREATE TABLE IF NOT EXISTS room_invites (
                token_hash     TEXT PRIMARY KEY,
                room_id        TEXT NOT NULL,
                created_by_key TEXT NOT NULL,
                single_use     INTEGER NOT NULL,
                redeemed_count INTEGER NOT NULL DEFAULT 0,
                revoked        INTEGER NOT NULL DEFAULT 0,
                created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_room_invites_room ON room_invites(room_id);

            CREATE TABLE IF NOT EXISTS room_grants (
                api_key TEXT NOT NULL,
                room_id TEXT NOT NULL,
                PRIMARY KEY (api_key, room_id)
            );
            ",
        )?;

        // Step 2: Run migrations (add columns to tables that may have been created by older versions)
        Self::migrate_add_column(
            &conn,
            "rooms",
            "visibility",
            "TEXT NOT NULL DEFAULT 'private'",
        );
        Self::migrate_add_column(&conn, "rooms", "owner_key", "TEXT");
        Self::migrate_add_column(&conn, "rooms", "encrypted", "INTEGER NOT NULL DEFAULT 0");
        ensure_column_exists(
            &conn,
            "votes",
            "eligible_voters",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_column_exists(&conn, "messages", "seq", "INTEGER NOT NULL DEFAULT 0")?;
        // Backfill seq for any existing rows that predate the column. rowid order
        // approximates insertion order, so this gives stable per-room seqs.
        backfill_message_seq(&conn)?;
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_messages_room_seq ON messages(room_id, seq);",
        )?;

        // Step 3: Seed data (runs after migrations so visibility column is guaranteed to exist)
        conn.execute_batch(
            "INSERT OR IGNORE INTO rooms (room_id, name, description, visibility)
                VALUES ('lobby', 'lobby', 'Default room for all agents', 'public');",
        )?;

        // Ensure lobby is public (may have been created before visibility existed)
        conn.execute(
            "UPDATE rooms SET visibility = 'public' WHERE room_id = 'lobby' AND visibility = 'private'",
            [],
        )?;

        // Seed the durable per-room sequence high-water from existing messages.
        // MAX() is used only during migration; normal allocation never derives
        // from retained rows, so a full retention purge cannot reset cursors.
        conn.execute_batch(
            "INSERT INTO room_sequences (room_id, high_water)
             SELECT r.room_id, COALESCE(MAX(m.seq), 0)
             FROM rooms r LEFT JOIN messages m ON m.room_id = r.room_id
             GROUP BY r.room_id
             ON CONFLICT(room_id) DO UPDATE SET
                 high_water = MAX(room_sequences.high_water, excluded.high_water);",
        )?;

        // Older binaries calculate seq from retained message rows. If one is
        // started after a full purge, fail its regressed insert instead of
        // silently reusing a sequence at/below the durable high-water. The
        // AFTER trigger also bridges safe old-binary inserts into the counter.
        conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS prevent_message_seq_regression
             BEFORE INSERT ON messages
             WHEN NEW.seq <= COALESCE(
                 (SELECT high_water FROM room_sequences WHERE room_id = NEW.room_id), 0
             )
             BEGIN
                 SELECT RAISE(ABORT, 'message seq below durable high-water');
             END;
             CREATE TRIGGER IF NOT EXISTS advance_message_seq_high_water
             AFTER INSERT ON messages
             BEGIN
                 INSERT INTO room_sequences (room_id, high_water) VALUES (NEW.room_id, NEW.seq)
                 ON CONFLICT(room_id) DO UPDATE SET
                     high_water = MAX(room_sequences.high_water, NEW.seq);
             END;",
        )?;

        // Legacy open votes stored only an eligible count. Preserve the known
        // electorate (creator plus any already-cast ballots) and fail closed to
        // that reconstructable set rather than admitting replacement agents.
        conn.execute_batch(
            "INSERT OR IGNORE INTO vote_eligible_agents (vote_id, agent_id)
             SELECT vote_id, created_by FROM votes WHERE status = 'open';
             INSERT OR IGNORE INTO vote_eligible_agents (vote_id, agent_id)
             SELECT b.vote_id, b.agent_id FROM vote_ballots b
             JOIN votes v ON v.vote_id = b.vote_id WHERE v.status = 'open';
             UPDATE votes SET eligible_voters = (
                 SELECT COUNT(*) FROM vote_eligible_agents e WHERE e.vote_id = votes.vote_id
             ) WHERE status = 'open';
             UPDATE votes SET status = 'closed'
             WHERE status = 'open' AND eligible_voters > 0
               AND (SELECT COUNT(*) FROM vote_ballots b WHERE b.vote_id = votes.vote_id)
                   >= eligible_voters;",
        )?;

        // Hydrate the in-memory grants cache from durable room_grants so the
        // broadcast hot path never queries SQLite.
        {
            let mut stmt = conn.prepare("SELECT api_key, room_id FROM room_grants")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (api_key, room_id) = row?;
                self.grants.entry(api_key).or_default().insert(room_id);
            }
        }

        Ok(())
    }

    /// Try to add a column to a table; silently ignore if it already exists.
    fn migrate_add_column(conn: &Connection, table: &str, column: &str, col_type: &str) {
        let sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, col_type);
        if let Err(e) = conn.execute_batch(&sql) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                log::debug!("Migration {}.{}: {}", table, column, msg);
            }
        }
    }

    // --- Room operations ---

    pub fn create_room(
        &self,
        room_id: &str,
        name: &str,
        description: Option<&str>,
        parent_id: Option<&str>,
        created_by: Option<&str>,
    ) -> Result<Room, StoreError> {
        self.create_room_with_visibility(
            room_id,
            name,
            description,
            parent_id,
            created_by,
            "private",
            None,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_room_with_visibility(
        &self,
        room_id: &str,
        name: &str,
        description: Option<&str>,
        parent_id: Option<&str>,
        created_by: Option<&str>,
        visibility: &str,
        owner_key: Option<&str>,
        encrypted: bool,
    ) -> Result<Room, StoreError> {
        let name = normalize_room_name(name).map_err(StoreError::InvalidRoomName)?;
        // SQL NULL predates durable ownership metadata and must remain
        // fail-closed. An empty string is the durable, distinguishable marker
        // for a room explicitly created by a keyless local connection.
        let persisted_owner_key = owner_key.unwrap_or("");
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO rooms (room_id, name, description, parent_id, created_by, visibility, owner_key, encrypted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![room_id, &name, description, parent_id, created_by, visibility, persisted_owner_key, encrypted],
        ).map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _) if err.extended_code == 2067 => {
                StoreError::RoomNameTaken(name.clone())
            }
            other => StoreError::Db(other),
        })?;

        // Query the created room inline (avoid deadlock from calling self.get_room)
        query_room_by_id(&conn, room_id)?
            .ok_or_else(|| StoreError::Db(rusqlite::Error::QueryReturnedNoRows))
    }

    pub fn get_room(&self, room_id: &str) -> Result<Option<Room>, StoreError> {
        let conn = self.conn.lock().unwrap();
        query_room_by_id(&conn, room_id)
    }

    pub fn get_room_by_name(&self, name: &str) -> Result<Option<Room>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT room_id, name, description, parent_id, created_by, created_at, visibility, owner_key, encrypted FROM rooms WHERE name = ?1",
        )?;

        let room = stmt.query_row(params![name], map_room_row);

        match room {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Db(e)),
        }
    }

    pub fn list_rooms(&self, parent_id: Option<&str>) -> Result<Vec<Room>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut rooms = Vec::new();

        match parent_id {
            Some(pid) => {
                let mut stmt = conn.prepare(
                    "SELECT room_id, name, description, parent_id, created_by, created_at, visibility, owner_key, encrypted FROM rooms WHERE parent_id = ?1 ORDER BY name",
                )?;
                let rows = stmt.query_map(params![pid], map_room_row)?;
                for row in rows {
                    rooms.push(row?);
                }
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT room_id, name, description, parent_id, created_by, created_at, visibility, owner_key, encrypted FROM rooms ORDER BY name",
                )?;
                let rows = stmt.query_map([], map_room_row)?;
                for row in rows {
                    rooms.push(row?);
                }
            }
        }

        Ok(rooms)
    }

    /// Authorize and persist a room rename in one immediate transaction.
    /// The owning bearer principal and its recorded creator attribution are
    /// both required.
    pub fn rename_room_authorized(
        &self,
        room_id: &str,
        agent_id: &str,
        agent_api_key: &str,
        no_auth: bool,
        name: &str,
    ) -> Result<Room, RenameRoomError> {
        let name = normalize_room_name(name).map_err(RenameRoomError::InvalidName)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut room = match tx.query_row(
            "SELECT room_id, name, description, parent_id, created_by, created_at, visibility, owner_key, encrypted
             FROM rooms WHERE room_id = ?1",
            params![room_id],
            map_room_row,
        ) {
            Ok(room) => room,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Err(RenameRoomError::NotFound),
            Err(error) => return Err(RenameRoomError::Db(error)),
        };

        if room.room_id == "lobby" || room.created_by.is_none() {
            return Err(RenameRoomError::ProtectedRoom);
        }
        if room.created_by.as_deref() != Some(agent_id)
            || !matches_room_owner(room.owner_key.as_deref(), agent_api_key, no_auth)
        {
            return Err(RenameRoomError::AccessDenied);
        }

        if room.name != name {
            tx.execute(
                "UPDATE rooms SET name = ?1 WHERE room_id = ?2",
                params![&name, room_id],
            )
            .map_err(|error| match error {
                rusqlite::Error::SqliteFailure(error, _) if error.extended_code == 2067 => {
                    RenameRoomError::NameTaken(name.clone())
                }
                other => RenameRoomError::Db(other),
            })?;
            room.name = name;
        }
        tx.commit()?;
        Ok(room)
    }

    /// Authorize and irreversibly remove a persisted room from active Cowchat
    /// state in one immediate transaction. Keeping the room lookup,
    /// creator/key checks, and deletes under the same SQLite write lock avoids
    /// a check/delete TOCTOU gap. This is not a forensic-erasure guarantee.
    pub fn destroy_room_authorized(
        &self,
        room_id: &str,
        agent_id: &str,
        agent_api_key: &str,
        no_auth: bool,
    ) -> Result<Room, DestroyRoomError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let room = match tx.query_row(
            "SELECT room_id, name, description, parent_id, created_by, created_at, visibility, owner_key, encrypted
             FROM rooms WHERE room_id = ?1",
            params![room_id],
            map_room_row,
        ) {
            Ok(room) => room,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Err(DestroyRoomError::NotFound),
            Err(error) => return Err(DestroyRoomError::Db(error)),
        };

        // Lobby and any other server-created/legacy system room have no
        // creator and are intentionally immutable through the public protocol.
        if room.room_id == "lobby" || room.created_by.is_none() {
            return Err(DestroyRoomError::ProtectedRoom);
        }

        // The API key (or local keyless boundary) is the bearer authority. The
        // exact creator ID is an attribution guard within that boundary, not a
        // separate unforgeable principal; reconnect semantics deliberately let
        // the bearer assume IDs owned by it. Public visibility never weakens
        // this mutation check.
        if room.created_by.as_deref() != Some(agent_id)
            || !matches_room_owner(room.owner_key.as_deref(), agent_api_key, no_auth)
        {
            return Err(DestroyRoomError::AccessDenied);
        }

        // Delete every room-scoped durable row explicitly. Several older
        // schemas predate room foreign keys on votes/subscriptions, so relying
        // only on ON DELETE CASCADE would leave credentialed webhooks or active
        // coordination state behind after an upgrade.
        detach_children_and_delete_room_artifacts(&tx, room_id)?;
        let affected = tx.execute("DELETE FROM rooms WHERE room_id = ?1", params![room_id])?;
        if affected != 1 {
            return Err(DestroyRoomError::NotFound);
        }
        tx.commit()?;
        drop(conn);
        self.purge_room_grants_from_cache(room_id);
        Ok(room)
    }

    /// Delete durable side-state for a room whose destruction raced an
    /// in-flight operation. Legacy schemas may contain room-scoped rows
    /// without foreign keys.
    pub fn delete_room_artifacts(&self, room_id: &str) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        delete_room_artifacts_in_transaction(&tx, room_id)?;
        tx.commit()?;
        drop(conn);
        self.purge_room_grants_from_cache(room_id);
        Ok(())
    }

    /// Delete persisted messages older than `age_modifier` (a SQLite datetime
    /// modifier such as "-14 days") in rooms whose owning key has tier `tier`.
    /// Rooms with no/unknown owner key are treated as 'free'. The cutoff is
    /// computed with the same `strftime` format the `created_at` default uses,
    /// so the comparison is an exact lexicographic match. Returns rows deleted.
    pub fn purge_messages_by_tier(
        &self,
        tier: &str,
        age_modifier: &str,
    ) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "DELETE FROM messages WHERE message_id IN (
                SELECT m.message_id FROM messages m
                JOIN rooms r ON m.room_id = r.room_id
                LEFT JOIN api_keys k ON r.owner_key = k.api_key
                WHERE COALESCE(k.tier, 'free') = ?1
                  AND m.created_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?2)
            )",
            params![tier, age_modifier],
        )?;
        Ok(affected)
    }

    // --- Message operations ---

    pub fn insert_message(
        &self,
        message_id: &str,
        room_id: &str,
        agent_id: &str,
        agent_name: &str,
        content: &str,
        reply_to_message: Option<&str>,
        metadata: &serde_json::Value,
    ) -> Result<ChatMessage, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let metadata_str = serde_json::to_string(metadata).unwrap_or_default();

        // Allocate and insert in one transaction. The AFTER INSERT trigger
        // advances high-water in the same transaction, so a failed insert cannot
        // consume a seq. The counter is independent of retained message rows.
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO room_sequences (room_id, high_water) VALUES (?1, 0)",
            params![room_id],
        )?;
        let seq: i64 = tx.query_row(
            "SELECT high_water + 1 FROM room_sequences WHERE room_id = ?1",
            params![room_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO messages (message_id, room_id, agent_id, agent_name, content, reply_to_message, metadata, seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![message_id, room_id, agent_id, agent_name, content, reply_to_message, metadata_str, seq],
        )?;

        let created_at: String = tx.query_row(
            "SELECT created_at FROM messages WHERE message_id = ?1",
            params![message_id],
            |row| row.get(0),
        )?;
        tx.commit()?;

        Ok(ChatMessage {
            message_id: message_id.to_string(),
            room_id: room_id.to_string(),
            agent_id: agent_id.to_string(),
            agent_name: agent_name.to_string(),
            content: content.to_string(),
            reply_to_message: reply_to_message.map(String::from),
            metadata: metadata.clone(),
            timestamp: parse_timestamp(&created_at),
            seq,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn accept_handoff(
        &self,
        message_id: &str,
        room_id: &str,
        agent_id: &str,
        agent_name: &str,
        content: &str,
        handoff_message_id: &str,
        note: Option<&str>,
    ) -> Result<ChatMessage, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        let target_metadata: String = tx
            .query_row(
                "SELECT metadata FROM messages WHERE room_id = ?1 AND message_id = ?2",
                params![room_id, handoff_message_id],
                |row| row.get(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => StoreError::HandoffNotFound,
                other => StoreError::Db(other),
            })?;
        let target_metadata: serde_json::Value =
            serde_json::from_str(&target_metadata).map_err(|_| StoreError::InvalidHandoff)?;
        let target_packet = target_metadata
            .get("handoff")
            .cloned()
            .and_then(|value| serde_json::from_value::<HandoffPacket>(value).ok())
            .filter(|packet| packet.validate().is_ok());
        if target_metadata.get("kind").and_then(|value| value.as_str()) != Some(HANDOFF_READY_KIND)
            || target_packet.is_none()
        {
            return Err(StoreError::InvalidHandoff);
        }

        {
            let mut statement = tx.prepare(
                "SELECT metadata FROM messages WHERE room_id = ?1 AND seq > 0 ORDER BY seq ASC",
            )?;
            let rows = statement.query_map(params![room_id], |row| row.get::<_, String>(0))?;
            for row in rows {
                let Ok(metadata) = serde_json::from_str::<serde_json::Value>(&row?) else {
                    continue;
                };
                match metadata.get("kind").and_then(|value| value.as_str()) {
                    Some(HANDOFF_ACCEPTED_KIND) => {
                        let acceptance = metadata
                            .get("handoff")
                            .cloned()
                            .and_then(|value| {
                                serde_json::from_value::<HandoffAcceptancePacket>(value).ok()
                            })
                            .filter(|packet| packet.validate().is_ok());
                        if acceptance
                            .is_some_and(|packet| packet.accepted_handoff_id == handoff_message_id)
                        {
                            return Err(StoreError::HandoffAlreadyAccepted);
                        }
                    }
                    Some(HANDOFF_READY_KIND) => {
                        let packet = metadata
                            .get("handoff")
                            .cloned()
                            .and_then(|value| serde_json::from_value::<HandoffPacket>(value).ok())
                            .filter(|packet| packet.validate().is_ok());
                        if packet.is_some_and(|packet| {
                            packet.supersedes.as_deref() == Some(handoff_message_id)
                        }) {
                            return Err(StoreError::HandoffSuperseded);
                        }
                    }
                    _ => {}
                }
            }
        }

        let acceptance = HandoffAcceptancePacket {
            version: HANDOFF_SCHEMA_VERSION,
            accepted_handoff_id: handoff_message_id.to_string(),
            note: note.map(str::to_string),
        };
        acceptance
            .validate()
            .map_err(|_| StoreError::InvalidHandoff)?;
        let metadata = serde_json::json!({
            "kind": HANDOFF_ACCEPTED_KIND,
            "handoff": acceptance,
        });
        let metadata_str = serde_json::to_string(&metadata).unwrap_or_default();

        tx.execute(
            "INSERT OR IGNORE INTO room_sequences (room_id, high_water) VALUES (?1, 0)",
            params![room_id],
        )?;
        let seq: i64 = tx.query_row(
            "SELECT high_water + 1 FROM room_sequences WHERE room_id = ?1",
            params![room_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO messages (message_id, room_id, agent_id, agent_name, content, reply_to_message, metadata, seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                message_id,
                room_id,
                agent_id,
                agent_name,
                content,
                handoff_message_id,
                metadata_str,
                seq
            ],
        )?;
        let created_at: String = tx.query_row(
            "SELECT created_at FROM messages WHERE message_id = ?1",
            params![message_id],
            |row| row.get(0),
        )?;
        tx.commit()?;

        Ok(ChatMessage {
            message_id: message_id.to_string(),
            room_id: room_id.to_string(),
            agent_id: agent_id.to_string(),
            agent_name: agent_name.to_string(),
            content: content.to_string(),
            reply_to_message: Some(handoff_message_id.to_string()),
            metadata,
            timestamp: parse_timestamp(&created_at),
            seq,
        })
    }

    pub fn validate_handoff_supersession(
        &self,
        room_id: &str,
        packet: &HandoffPacket,
    ) -> Result<(), StoreError> {
        let Some(superseded_id) = packet.supersedes.as_deref() else {
            return Ok(());
        };
        let conn = self.conn.lock().unwrap();
        let metadata: String = conn
            .query_row(
                "SELECT metadata FROM messages WHERE room_id = ?1 AND message_id = ?2",
                params![room_id, superseded_id],
                |row| row.get(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => StoreError::HandoffNotFound,
                other => StoreError::Db(other),
            })?;
        let metadata: serde_json::Value =
            serde_json::from_str(&metadata).map_err(|_| StoreError::InvalidHandoff)?;
        if metadata.get("kind").and_then(|value| value.as_str()) != Some(HANDOFF_READY_KIND) {
            return Err(StoreError::InvalidHandoff);
        }
        let superseded = metadata
            .get("handoff")
            .cloned()
            .and_then(|value| serde_json::from_value::<HandoffPacket>(value).ok())
            .filter(|packet| packet.validate().is_ok())
            .ok_or(StoreError::InvalidHandoff)?;
        if superseded.task_id != packet.task_id {
            return Err(StoreError::HandoffTaskMismatch {
                superseded_task: superseded.task_id,
                replacement_task: packet.task_id.clone(),
            });
        }
        Ok(())
    }

    /// Returns the durable high-water seq for a room, or 0 if it has never had
    /// a persisted message. Retention does not lower this value.
    pub fn room_tip(&self, room_id: &str) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap();
        let seq: i64 = conn.query_row(
            "SELECT COALESCE((SELECT high_water FROM room_sequences WHERE room_id = ?1), 0)",
            params![room_id],
            |row| row.get(0),
        )?;
        Ok(seq)
    }

    pub fn get_history(
        &self,
        room_id: &str,
        limit: u32,
        before: Option<DateTime<Utc>>,
    ) -> Result<Vec<ChatMessage>, StoreError> {
        self.get_history_filtered(room_id, limit, before, None, None)
    }

    /// Backwards-compatible wrapper for the message_id-based `since` filter.
    pub fn get_history_since(
        &self,
        room_id: &str,
        limit: u32,
        before: Option<DateTime<Utc>>,
        since: Option<&str>,
    ) -> Result<Vec<ChatMessage>, StoreError> {
        self.get_history_filtered(room_id, limit, before, since, None)
    }

    /// Get message history with optional filters.
    /// `since_seq` takes precedence over `since` (message_id), which takes precedence over `before`.
    /// All ASC-ordered queries return chronological order; `before` returns the latest N before
    /// the cutoff, also chronologically ordered.
    pub fn get_history_filtered(
        &self,
        room_id: &str,
        limit: u32,
        before: Option<DateTime<Utc>>,
        since: Option<&str>,
        since_seq: Option<i64>,
    ) -> Result<Vec<ChatMessage>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut messages = Vec::new();

        if let Some(seq_floor) = since_seq {
            let mut stmt = conn.prepare(
                "SELECT message_id, room_id, agent_id, agent_name, content, reply_to_message, metadata, created_at, seq
                 FROM messages WHERE room_id = ?1 AND seq > ?2
                 ORDER BY seq ASC LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![room_id, seq_floor, limit], map_message_row)?;
            for row in rows {
                messages.push(row?);
            }
            return Ok(messages);
        }

        if let Some(since_id) = since {
            // Get the rowid of the since message, then return messages after it
            let mut stmt = conn.prepare(
                "SELECT message_id, room_id, agent_id, agent_name, content, reply_to_message, metadata, created_at, seq
                 FROM messages WHERE room_id = ?1 AND rowid > (
                     SELECT rowid FROM messages WHERE message_id = ?2
                 )
                 ORDER BY created_at ASC, rowid ASC LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![room_id, since_id, limit], map_message_row)?;
            for row in rows {
                messages.push(row?);
            }
            // Already in chronological order
            return Ok(messages);
        }

        match before {
            Some(before_ts) => {
                let ts_str = before_ts.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
                let mut stmt = conn.prepare(
                    "SELECT message_id, room_id, agent_id, agent_name, content, reply_to_message, metadata, created_at, seq
                     FROM messages WHERE room_id = ?1 AND created_at < ?2
                     ORDER BY created_at DESC, rowid DESC LIMIT ?3",
                )?;
                let rows = stmt.query_map(params![room_id, ts_str, limit], map_message_row)?;
                for row in rows {
                    messages.push(row?);
                }
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT message_id, room_id, agent_id, agent_name, content, reply_to_message, metadata, created_at, seq
                     FROM messages WHERE room_id = ?1
                     ORDER BY created_at DESC, rowid DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![room_id, limit], map_message_row)?;
                for row in rows {
                    messages.push(row?);
                }
            }
        }

        // Return in chronological order
        messages.reverse();
        Ok(messages)
    }

    // --- Agent session tracking ---

    pub fn record_session_start(
        &self,
        session_id: &str,
        agent_id: &str,
        agent_name: &str,
        capabilities: &[String],
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let caps_json = serde_json::to_string(capabilities).unwrap_or_default();
        conn.execute(
            "INSERT INTO agent_sessions (session_id, agent_id, agent_name, capabilities) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, agent_id, agent_name, caps_json],
        )?;
        Ok(())
    }

    pub fn record_session_end(&self, session_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE agent_sessions SET disconnected_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// Permanently bind a public agent id to the credential that first claims
    /// it. Reconnect stash expiry and server restarts must not erase ownership.
    pub fn claim_agent_identity(
        &self,
        agent_id: &str,
        owner_key: &str,
    ) -> Result<bool, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO agent_identities (agent_id, owner_key) VALUES (?1, ?2)",
            params![agent_id, owner_key],
        )?;
        let stored_key: String = tx.query_row(
            "SELECT owner_key FROM agent_identities WHERE agent_id = ?1",
            params![agent_id],
            |row| row.get(0),
        )?;
        tx.commit()?;
        Ok(stored_key == owner_key)
    }

    // --- Vote operations ---

    pub fn create_vote(
        &self,
        vote_id: &str,
        room_id: &str,
        title: &str,
        description: Option<&str>,
        options: &[String],
        created_by: &str,
        closes_at: Option<DateTime<Utc>>,
        eligible_agents: &[String],
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let options_json = serde_json::to_string(options).unwrap_or_default();
        let closes_str = closes_at.map(|t| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string());
        tx.execute(
            "INSERT INTO votes (vote_id, room_id, title, description, options, created_by, closes_at, eligible_voters) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![vote_id, room_id, title, description, options_json, created_by, closes_str, eligible_agents.len() as i64],
        )?;
        for agent_id in eligible_agents {
            tx.execute(
                "INSERT INTO vote_eligible_agents (vote_id, agent_id) VALUES (?1, ?2)",
                params![vote_id, agent_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_vote_eligible_agents(&self, vote_id: &str) -> Result<Vec<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent_id FROM vote_eligible_agents WHERE vote_id = ?1 ORDER BY agent_id",
        )?;
        let rows = stmt.query_map(params![vote_id], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::Db)
    }

    pub fn cast_vote(
        &self,
        vote_id: &str,
        agent_id: &str,
        agent_name: &str,
        option_index: usize,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();

        // Check vote is open
        let status: String = conn
            .query_row(
                "SELECT status FROM votes WHERE vote_id = ?1",
                params![vote_id],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => StoreError::VoteNotFound,
                other => StoreError::Db(other),
            })?;

        if status != "open" {
            return Err(StoreError::VoteClosed);
        }

        conn.execute(
            "INSERT INTO vote_ballots (vote_id, agent_id, agent_name, option_index) VALUES (?1, ?2, ?3, ?4)",
            params![vote_id, agent_id, agent_name, option_index as i64],
        ).map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _) if err.extended_code == 1555 => {
                StoreError::AlreadyVoted
            }
            other => StoreError::Db(other),
        })?;

        Ok(())
    }

    pub fn get_vote_ballot_count(&self, vote_id: &str) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM vote_ballots WHERE vote_id = ?1",
            params![vote_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn get_vote_ballots(
        &self,
        vote_id: &str,
    ) -> Result<Vec<(String, String, usize)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent_id, agent_name, option_index FROM vote_ballots WHERE vote_id = ?1 ORDER BY cast_at",
        )?;
        let rows = stmt.query_map(params![vote_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? as usize,
            ))
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn save_task(&self, task: &cowchat_core::TaskInfo) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO room_tasks
             (task_id, room_id, title, description, status, assignee, created_by, created_at, updated_at, note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(task_id) DO UPDATE SET
               status=excluded.status, assignee=excluded.assignee,
               updated_at=excluded.updated_at, note=excluded.note",
            params![
                task.task_id,
                task.room_id,
                task.title,
                task.description,
                task.status,
                task.assignee,
                task.created_by,
                task.created_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
                task.updated_at.map(|value| value.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()),
                task.note,
            ],
        )?;
        Ok(())
    }

    pub fn load_tasks(&self) -> Result<Vec<cowchat_core::TaskInfo>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT task_id, room_id, title, description, status, assignee,
                    created_by, created_at, updated_at, note FROM room_tasks",
        )?;
        let rows = stmt.query_map([], |row| {
            let created_at: String = row.get(7)?;
            let updated_at: Option<String> = row.get(8)?;
            Ok(cowchat_core::TaskInfo {
                task_id: row.get(0)?,
                room_id: row.get(1)?,
                title: row.get(2)?,
                description: row.get(3)?,
                status: row.get(4)?,
                assignee: row.get(5)?,
                created_by: row.get(6)?,
                created_at: parse_timestamp(&created_at),
                updated_at: updated_at.as_deref().map(parse_timestamp),
                note: row.get(9)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::Db)
    }

    pub fn close_vote(&self, vote_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE votes SET status = 'closed' WHERE vote_id = ?1",
            params![vote_id],
        )?;
        Ok(())
    }

    // --- API key operations ---

    pub fn create_api_key(&self, api_key: &str, label: Option<&str>) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO api_keys (api_key, label) VALUES (?1, ?2)",
            params![api_key, label],
        )?;
        Ok(())
    }

    pub fn validate_api_key(&self, api_key: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM api_keys WHERE api_key = ?1",
            params![api_key],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn get_key_tier(&self, api_key: &str) -> Result<String, StoreError> {
        let conn = self.conn.lock().unwrap();
        let tier: String = conn
            .query_row(
                "SELECT tier FROM api_keys WHERE api_key = ?1",
                params![api_key],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => StoreError::Db(e),
                other => StoreError::Db(other),
            })?;
        Ok(tier)
    }

    // --- Room invite operations ---

    /// Persist a new invite. Only the SHA-256 hex of the token is stored; the
    /// raw token exists solely in the creation reply.
    pub fn create_invite(
        &self,
        token_hash: &str,
        room_id: &str,
        created_by_key: &str,
        single_use: bool,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO room_invites (token_hash, room_id, created_by_key, single_use)
             VALUES (?1, ?2, ?3, ?4)",
            params![token_hash, room_id, created_by_key, single_use],
        )?;
        Ok(())
    }

    pub fn get_invite(&self, token_hash: &str) -> Result<Option<InviteMeta>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT room_id, created_by_key, single_use, redeemed_count, revoked
             FROM room_invites WHERE token_hash = ?1",
            params![token_hash],
            |row| {
                Ok(InviteMeta {
                    room_id: row.get(0)?,
                    created_by_key: row.get(1)?,
                    single_use: row.get::<_, i64>(2)? != 0,
                    redeemed_count: row.get(3)?,
                    revoked: row.get::<_, i64>(4)? != 0,
                })
            },
        );
        match result {
            Ok(meta) => Ok(Some(meta)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Db(e)),
        }
    }

    /// Atomically consume one redemption. The guarded UPDATE is the whole
    /// race: of two concurrent redeems of a single-use invite exactly one
    /// matches `revoked = 0` and wins; the loser sees zero rows affected.
    /// Returns the invite's room_id on success, None for unknown/revoked/
    /// used-up tokens (callers must not distinguish those cases).
    pub fn redeem_invite(&self, token_hash: &str) -> Result<Option<String>, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let affected = tx.execute(
            "UPDATE room_invites
             SET redeemed_count = redeemed_count + 1,
                 revoked = CASE WHEN single_use != 0 THEN 1 ELSE revoked END
             WHERE token_hash = ?1 AND revoked = 0",
            params![token_hash],
        )?;
        if affected == 0 {
            return Ok(None);
        }
        let room_id: String = tx.query_row(
            "SELECT room_id FROM room_invites WHERE token_hash = ?1",
            params![token_hash],
            |row| row.get(0),
        )?;
        tx.commit()?;
        Ok(Some(room_id))
    }

    /// Mark an invite revoked so no further redemption succeeds. Idempotent;
    /// returns false only when the token hash is unknown. Keys already minted
    /// through the invite keep their grants.
    pub fn revoke_invite(&self, token_hash: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "UPDATE room_invites SET revoked = 1 WHERE token_hash = ?1",
            params![token_hash],
        )?;
        Ok(affected > 0)
    }

    /// Record that `api_key` may access `room_id`, durably and in the cache.
    pub fn add_room_grant(&self, api_key: &str, room_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO room_grants (api_key, room_id) VALUES (?1, ?2)",
            params![api_key, room_id],
        )?;
        drop(conn);
        self.grants
            .entry(api_key.to_string())
            .or_default()
            .insert(room_id.to_string());
        Ok(())
    }

    /// Cache-only grant check — safe on the per-broadcast hot path.
    pub fn key_has_grant(&self, api_key: &str, room_id: &str) -> bool {
        !api_key.is_empty()
            && self
                .grants
                .get(api_key)
                .is_some_and(|rooms| rooms.contains(room_id))
    }

    /// All API keys holding a grant for `room_id` (cache scan; used on the
    /// rare room-metadata broadcast paths, not per message).
    pub fn granted_keys_for_room(&self, room_id: &str) -> HashSet<String> {
        self.grants
            .iter()
            .filter(|entry| entry.value().contains(room_id))
            .map(|entry| entry.key().clone())
            .collect()
    }

    fn purge_room_grants_from_cache(&self, room_id: &str) {
        self.grants.retain(|_, rooms| {
            rooms.remove(room_id);
            !rooms.is_empty()
        });
    }

    pub fn count_rooms_for_key(&self, api_key: &str) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM rooms WHERE owner_key = ?1",
            params![api_key],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// List rooms visible to the given API key: all public rooms + private
    /// rooms owned by the key + private rooms the key holds a grant for.
    pub fn list_rooms_for_key(
        &self,
        api_key: Option<&str>,
        parent_id: Option<&str>,
    ) -> Result<Vec<Room>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut rooms = Vec::new();

        match (parent_id, api_key) {
            (Some(pid), Some(key)) => {
                let mut stmt = conn.prepare(
                    "SELECT room_id, name, description, parent_id, created_by, created_at, visibility, owner_key, encrypted
                     FROM rooms WHERE parent_id = ?1 AND (visibility = 'public' OR owner_key = ?2
                         OR room_id IN (SELECT room_id FROM room_grants WHERE api_key = ?2)) ORDER BY name",
                )?;
                let rows = stmt.query_map(params![pid, key], map_room_row)?;
                for row in rows {
                    rooms.push(row?);
                }
            }
            (Some(pid), None) => {
                let mut stmt = conn.prepare(
                    "SELECT room_id, name, description, parent_id, created_by, created_at, visibility, owner_key, encrypted
                     FROM rooms WHERE parent_id = ?1 AND visibility = 'public' ORDER BY name",
                )?;
                let rows = stmt.query_map(params![pid], map_room_row)?;
                for row in rows {
                    rooms.push(row?);
                }
            }
            (None, Some(key)) => {
                let mut stmt = conn.prepare(
                    "SELECT room_id, name, description, parent_id, created_by, created_at, visibility, owner_key, encrypted
                     FROM rooms WHERE visibility = 'public' OR owner_key = ?1
                         OR room_id IN (SELECT room_id FROM room_grants WHERE api_key = ?1) ORDER BY name",
                )?;
                let rows = stmt.query_map(params![key], map_room_row)?;
                for row in rows {
                    rooms.push(row?);
                }
            }
            (None, None) => {
                let mut stmt = conn.prepare(
                    "SELECT room_id, name, description, parent_id, created_by, created_at, visibility, owner_key, encrypted
                     FROM rooms WHERE visibility = 'public' ORDER BY name",
                )?;
                let rows = stmt.query_map([], map_room_row)?;
                for row in rows {
                    rooms.push(row?);
                }
            }
        }

        Ok(rooms)
    }

    // --- Vote operations (continued) ---

    pub fn get_vote_meta(&self, vote_id: &str) -> Result<Option<VoteMeta>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT vote_id, room_id, title, description, options, created_by, created_at, closes_at, status, eligible_voters FROM votes WHERE vote_id = ?1",
            params![vote_id],
            map_vote_meta_row,
        );

        match result {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Db(e)),
        }
    }

    pub fn list_votes(&self, room_id: &str, limit: u32) -> Result<Vec<VoteMeta>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT vote_id, room_id, title, description, options, created_by, created_at, closes_at, status, eligible_voters
             FROM votes
             WHERE room_id = ?1
             ORDER BY created_at DESC, vote_id DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![room_id, limit as i64], map_vote_meta_row)?;
        let mut votes = Vec::new();
        for row in rows {
            votes.push(row?);
        }
        Ok(votes)
    }

    pub fn list_open_votes(&self) -> Result<Vec<VoteMeta>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT vote_id, room_id, title, description, options, created_by,
                    created_at, closes_at, status, eligible_voters
             FROM votes WHERE status = 'open' ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], map_vote_meta_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::Db)
    }

    // --- Subscription operations ---

    /// Insert a new subscription. `subscription_id` and `created_at` are caller-provided
    /// so we keep insert/read symmetric; callers should generate a UUID and `Utc::now()`.
    #[allow(clippy::too_many_arguments)]
    pub fn create_subscription(
        &self,
        subscription_id: &str,
        room_id: &str,
        owner_key: &str,
        webhook_url: &str,
        secret: &str,
        kinds: &[String],
        only_from: Option<&str>,
        not_from: Option<&str>,
        exclude_thinking: bool,
        since_seq: i64,
    ) -> Result<cowchat_core::Subscription, StoreError> {
        let conn = self.conn.lock().unwrap();
        let kinds_json = if kinds.is_empty() {
            None
        } else {
            Some(serde_json::to_string(kinds).unwrap_or_else(|_| "[]".to_string()))
        };
        let now = chrono::Utc::now();
        let now_str = now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        conn.execute(
            "INSERT INTO subscriptions
             (subscription_id, room_id, owner_key, webhook_url, secret, kinds,
              only_from, not_from, exclude_thinking, since_seq, last_delivered_seq,
              status, failure_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, 'active', 0, ?11)",
            params![
                subscription_id,
                room_id,
                owner_key,
                webhook_url,
                secret,
                kinds_json,
                only_from,
                not_from,
                exclude_thinking as i64,
                since_seq,
                now_str,
            ],
        )?;
        Ok(cowchat_core::Subscription {
            subscription_id: subscription_id.to_string(),
            room_id: room_id.to_string(),
            webhook_url: webhook_url.to_string(),
            kinds: kinds.to_vec(),
            only_from: only_from.map(String::from),
            not_from: not_from.map(String::from),
            exclude_thinking,
            since_seq,
            last_delivered_seq: since_seq,
            status: "active".to_string(),
            failure_count: 0,
            created_at: now,
        })
    }

    pub fn get_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<Option<(cowchat_core::Subscription, String, String)>, StoreError> {
        // Returns (sub, owner_key, secret) — secret is needed by the delivery worker
        // but not exposed to clients.
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT subscription_id, room_id, owner_key, webhook_url, secret, kinds,
                    only_from, not_from, exclude_thinking, since_seq, last_delivered_seq,
                    status, failure_count, created_at
             FROM subscriptions WHERE subscription_id = ?1",
        )?;
        let mut rows = stmt.query(params![subscription_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(map_subscription_row(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn list_subscriptions(
        &self,
        owner_key: &str,
        room_id: Option<&str>,
    ) -> Result<Vec<cowchat_core::Subscription>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let (sql, with_room) = match room_id {
            Some(_) => (
                "SELECT subscription_id, room_id, owner_key, webhook_url, secret, kinds,
                        only_from, not_from, exclude_thinking, since_seq, last_delivered_seq,
                        status, failure_count, created_at
                 FROM subscriptions
                 WHERE owner_key = ?1 AND room_id = ?2
                 ORDER BY created_at DESC",
                true,
            ),
            None => (
                "SELECT subscription_id, room_id, owner_key, webhook_url, secret, kinds,
                        only_from, not_from, exclude_thinking, since_seq, last_delivered_seq,
                        status, failure_count, created_at
                 FROM subscriptions
                 WHERE owner_key = ?1
                 ORDER BY created_at DESC",
                false,
            ),
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = if with_room {
            stmt.query(params![owner_key, room_id.unwrap()])?
        } else {
            stmt.query(params![owner_key])?
        };
        let mut out = Vec::new();
        let mut rows = rows;
        while let Some(row) = rows.next()? {
            let (sub, _owner, _secret) = map_subscription_row(row)?;
            out.push(sub);
        }
        Ok(out)
    }

    /// Return ALL active subscriptions for a room (used by the delivery enqueue path).
    pub fn list_active_subscriptions_for_room(
        &self,
        room_id: &str,
    ) -> Result<Vec<(cowchat_core::Subscription, String, String)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT subscription_id, room_id, owner_key, webhook_url, secret, kinds,
                    only_from, not_from, exclude_thinking, since_seq, last_delivered_seq,
                    status, failure_count, created_at
             FROM subscriptions WHERE room_id = ?1 AND status = 'active'",
        )?;
        let mut rows = stmt.query(params![room_id])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(map_subscription_row(row)?);
        }
        Ok(out)
    }

    pub fn delete_subscription(
        &self,
        subscription_id: &str,
        owner_key: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "DELETE FROM subscriptions WHERE subscription_id = ?1 AND owner_key = ?2",
            params![subscription_id, owner_key],
        )?;
        Ok(n > 0)
    }

    pub fn set_subscription_status(
        &self,
        subscription_id: &str,
        status: &str,
        failure_count: Option<i64>,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        match failure_count {
            Some(n) => {
                conn.execute(
                    "UPDATE subscriptions SET status = ?1, failure_count = ?2 WHERE subscription_id = ?3",
                    params![status, n, subscription_id],
                )?;
            }
            None => {
                conn.execute(
                    "UPDATE subscriptions SET status = ?1 WHERE subscription_id = ?2",
                    params![status, subscription_id],
                )?;
            }
        }
        Ok(())
    }

    pub fn advance_subscription_cursor(
        &self,
        subscription_id: &str,
        new_last_delivered_seq: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE subscriptions SET last_delivered_seq = ?1, failure_count = 0
             WHERE subscription_id = ?2 AND ?1 > last_delivered_seq",
            params![new_last_delivered_seq, subscription_id],
        )?;
        Ok(())
    }

    // --- Subscription delivery queue ---

    pub fn enqueue_delivery(
        &self,
        delivery_id: &str,
        subscription_id: &str,
        message_seq: i64,
        message_id: &str,
        next_attempt_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, StoreError> {
        // Returns true if a new row was inserted, false if a duplicate (sub, seq)
        // was rejected by the UNIQUE constraint (idempotent enqueue).
        let conn = self.conn.lock().unwrap();
        let ts = next_attempt_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        match conn.execute(
            "INSERT INTO subscription_deliveries
             (delivery_id, subscription_id, message_seq, message_id, next_attempt_at, attempts)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![delivery_id, subscription_id, message_seq, message_id, ts],
        ) {
            Ok(_) => Ok(true),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok(false)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Load pending deliveries due to fire by `now`. Limit caps batch size.
    pub fn load_due_deliveries(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        limit: usize,
    ) -> Result<Vec<PendingDelivery>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let ts = now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        let mut stmt = conn.prepare(
            "SELECT delivery_id, subscription_id, message_seq, message_id, attempts
             FROM subscription_deliveries AS candidate
             WHERE next_attempt_at <= ?1
               AND NOT EXISTS (
                   SELECT 1 FROM subscription_deliveries AS earlier
                   WHERE earlier.subscription_id = candidate.subscription_id
                     AND earlier.message_seq < candidate.message_seq
               )
             ORDER BY next_attempt_at ASC, message_seq ASC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![ts, limit as i64], |row| {
            Ok(PendingDelivery {
                delivery_id: row.get(0)?,
                subscription_id: row.get(1)?,
                message_seq: row.get(2)?,
                message_id: row.get(3)?,
                attempts: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Earliest `next_attempt_at` of any pending delivery, or None if the
    /// queue is empty. The delivery worker uses this to sleep precisely until
    /// the next retry instead of polling.
    pub fn earliest_pending_attempt(
        &self,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT next_attempt_at FROM subscription_deliveries
             ORDER BY next_attempt_at ASC LIMIT 1",
        )?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let s: String = row.get(0)?;
            let dt = chrono::DateTime::parse_from_rfc3339(&s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .ok();
            Ok(dt)
        } else {
            Ok(None)
        }
    }

    pub fn delete_delivery(&self, delivery_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM subscription_deliveries WHERE delivery_id = ?1",
            params![delivery_id],
        )?;
        Ok(())
    }

    pub fn reschedule_delivery(
        &self,
        delivery_id: &str,
        next_attempt_at: chrono::DateTime<chrono::Utc>,
        attempts: i64,
        last_error: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let ts = next_attempt_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        conn.execute(
            "UPDATE subscription_deliveries
             SET next_attempt_at = ?1, attempts = ?2, last_error = ?3
             WHERE delivery_id = ?4",
            params![ts, attempts, last_error, delivery_id],
        )?;
        Ok(())
    }

    /// Fetch a single message by id for the delivery worker.
    pub fn get_message(&self, message_id: &str) -> Result<Option<ChatMessage>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT message_id, room_id, agent_id, agent_name, content, reply_to_message, metadata, created_at, seq
             FROM messages WHERE message_id = ?1",
        )?;
        let mut rows = stmt.query_map(params![message_id], map_message_row)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PendingDelivery {
    pub delivery_id: String,
    pub subscription_id: String,
    pub message_seq: i64,
    pub message_id: String,
    pub attempts: i64,
}

fn map_subscription_row(
    row: &rusqlite::Row<'_>,
) -> Result<(cowchat_core::Subscription, String, String), rusqlite::Error> {
    let subscription_id: String = row.get(0)?;
    let room_id: String = row.get(1)?;
    let owner_key: String = row.get(2)?;
    let webhook_url: String = row.get(3)?;
    let secret: String = row.get(4)?;
    let kinds_json: Option<String> = row.get(5)?;
    let only_from: Option<String> = row.get(6)?;
    let not_from: Option<String> = row.get(7)?;
    let exclude_thinking: i64 = row.get(8)?;
    let since_seq: i64 = row.get(9)?;
    let last_delivered_seq: i64 = row.get(10)?;
    let status: String = row.get(11)?;
    let failure_count: i64 = row.get(12)?;
    let created_at_str: String = row.get(13)?;
    let kinds: Vec<String> = kinds_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());
    Ok((
        cowchat_core::Subscription {
            subscription_id,
            room_id,
            webhook_url,
            kinds,
            only_from,
            not_from,
            exclude_thinking: exclude_thinking != 0,
            since_seq,
            last_delivered_seq,
            status,
            failure_count,
            created_at,
        },
        owner_key,
        secret,
    ))
}

// --- Internal helpers that take an already-locked connection ---

fn delete_room_artifacts_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    room_id: &str,
) -> Result<(), rusqlite::Error> {
    tx.execute(
        "DELETE FROM subscription_deliveries
         WHERE subscription_id IN (
             SELECT subscription_id FROM subscriptions WHERE room_id = ?1
         )",
        params![room_id],
    )?;
    tx.execute(
        "DELETE FROM subscriptions WHERE room_id = ?1",
        params![room_id],
    )?;
    tx.execute(
        "DELETE FROM vote_ballots
         WHERE vote_id IN (SELECT vote_id FROM votes WHERE room_id = ?1)",
        params![room_id],
    )?;
    tx.execute(
        "DELETE FROM vote_eligible_agents
         WHERE vote_id IN (SELECT vote_id FROM votes WHERE room_id = ?1)",
        params![room_id],
    )?;
    tx.execute("DELETE FROM votes WHERE room_id = ?1", params![room_id])?;
    tx.execute(
        "DELETE FROM room_tasks WHERE room_id = ?1",
        params![room_id],
    )?;
    tx.execute("DELETE FROM messages WHERE room_id = ?1", params![room_id])?;
    tx.execute(
        "DELETE FROM room_sequences WHERE room_id = ?1",
        params![room_id],
    )?;
    tx.execute(
        "DELETE FROM room_invites WHERE room_id = ?1",
        params![room_id],
    )?;
    tx.execute(
        "DELETE FROM room_grants WHERE room_id = ?1",
        params![room_id],
    )?;
    Ok(())
}

fn detach_children_and_delete_room_artifacts(
    tx: &rusqlite::Transaction<'_>,
    room_id: &str,
) -> Result<(), rusqlite::Error> {
    tx.execute(
        "UPDATE rooms SET parent_id = NULL WHERE parent_id = ?1",
        params![room_id],
    )?;
    delete_room_artifacts_in_transaction(tx, room_id)
}

fn query_room_by_id(conn: &Connection, room_id: &str) -> Result<Option<Room>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT room_id, name, description, parent_id, created_by, created_at, visibility, owner_key, encrypted FROM rooms WHERE room_id = ?1",
    )?;

    let room = stmt.query_row(params![room_id], map_room_row);

    match room {
        Ok(r) => Ok(Some(r)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(StoreError::Db(e)),
    }
}

fn ensure_column_exists(
    conn: &Connection,
    table: &str,
    column: &str,
    column_sql: &str,
) -> Result<(), rusqlite::Error> {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&pragma)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;

    let mut exists = false;
    for row in rows {
        if row?.eq_ignore_ascii_case(column) {
            exists = true;
            break;
        }
    }

    if !exists {
        let alter = format!("ALTER TABLE {table} ADD COLUMN {column} {column_sql}");
        conn.execute(&alter, [])?;
    }

    Ok(())
}

fn map_room_row(row: &rusqlite::Row) -> rusqlite::Result<Room> {
    let persisted_owner_key: Option<String> = row.get(7)?;
    let owner_key = match persisted_owner_key {
        Some(key) if key.is_empty() => None,
        Some(key) => Some(key),
        None => Some(LEGACY_UNOWNED_OWNER_KEY.to_string()),
    };
    Ok(Room {
        room_id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        parent_id: row.get(3)?,
        created_at: parse_timestamp(&row.get::<_, String>(5)?),
        created_by: row.get(4)?,
        visibility: row
            .get::<_, String>(6)
            .unwrap_or_else(|_| "private".to_string()),
        owner_key,
        last_activity: None,
        member_count: None,
        encrypted: row.get::<_, bool>(8).unwrap_or(false),
    })
}

fn map_vote_meta_row(row: &rusqlite::Row) -> rusqlite::Result<VoteMeta> {
    let options_str: String = row.get(4)?;
    let closes_str: Option<String> = row.get(7)?;
    let created_str: String = row.get(6)?;

    Ok(VoteMeta {
        vote_id: row.get(0)?,
        room_id: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        options: serde_json::from_str(&options_str).unwrap_or_default(),
        created_by: row.get(5)?,
        created_at: parse_timestamp(&created_str),
        closes_at: closes_str.map(|s| parse_timestamp(&s)),
        status: row.get(8)?,
        eligible_voters: row.get::<_, i64>(9)? as usize,
    })
}

fn map_message_row(row: &rusqlite::Row) -> rusqlite::Result<ChatMessage> {
    let metadata_str: String = row.get(6)?;
    let metadata: serde_json::Value =
        serde_json::from_str(&metadata_str).unwrap_or(serde_json::json!({}));
    let ts_str: String = row.get(7)?;
    let seq: i64 = row.get(8).unwrap_or(0);

    Ok(ChatMessage {
        message_id: row.get(0)?,
        room_id: row.get(1)?,
        agent_id: row.get(2)?,
        agent_name: row.get(3)?,
        content: row.get(4)?,
        reply_to_message: row.get(5)?,
        metadata,
        timestamp: parse_timestamp(&ts_str),
        seq,
    })
}

/// One-time migration: assign per-room seq to any rows that still have seq=0.
/// Uses rowid order as a stand-in for insertion order. Idempotent.
fn backfill_message_seq(conn: &Connection) -> Result<(), rusqlite::Error> {
    // Skip if there are no zero-seq rows.
    let zero_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM messages WHERE seq = 0", [], |row| {
            row.get(0)
        })?;
    if zero_count == 0 {
        return Ok(());
    }

    conn.execute_batch(
        "WITH numbered AS (
            SELECT rowid, ROW_NUMBER() OVER (PARTITION BY room_id ORDER BY rowid) AS rn
            FROM messages WHERE seq = 0
        )
        UPDATE messages
            SET seq = (SELECT rn FROM numbered WHERE numbered.rowid = messages.rowid)
            WHERE rowid IN (SELECT rowid FROM numbered);",
    )?;
    Ok(())
}

fn parse_timestamp(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.fZ")
                .map(|ndt| ndt.and_utc())
        })
        .unwrap_or_else(|_| Utc::now())
}

#[derive(Debug, thiserror::Error)]
pub enum DestroyRoomError {
    #[error("room not found")]
    NotFound,

    #[error("access denied")]
    AccessDenied,

    #[error("system room cannot be destroyed")]
    ProtectedRoom,

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("durable room cleanup failed: {0}")]
    Cleanup(#[from] StoreError),
}

#[derive(Debug, thiserror::Error)]
pub enum RenameRoomError {
    #[error("room not found")]
    NotFound,

    #[error("access denied")]
    AccessDenied,

    #[error("system room cannot be renamed")]
    ProtectedRoom,

    #[error("invalid room name: {0}")]
    InvalidName(String),

    #[error("room name already taken: {0}")]
    NameTaken(String),

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("room name already taken: {0}")]
    RoomNameTaken(String),

    #[error("invalid room name: {0}")]
    InvalidRoomName(String),

    #[error("vote not found")]
    VoteNotFound,

    #[error("vote is closed")]
    VoteClosed,

    #[error("already voted")]
    AlreadyVoted,

    #[error("handoff not found")]
    HandoffNotFound,

    #[error("message is not a valid handoff")]
    InvalidHandoff,

    #[error("handoff was already accepted")]
    HandoffAlreadyAccepted,

    #[error("handoff was superseded")]
    HandoffSuperseded,

    #[error("superseded handoff belongs to task {superseded_task}, not {replacement_task}")]
    HandoffTaskMismatch {
        superseded_task: String,
        replacement_task: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_creates_lobby() {
        let store = Store::open_in_memory().unwrap();
        let room = store.get_room("lobby").unwrap();
        assert!(room.is_some());
        assert_eq!(room.unwrap().name, "lobby");
    }

    #[test]
    fn test_create_and_get_room() {
        let store = Store::open_in_memory().unwrap();
        let room = store
            .create_room(
                "test-room",
                "test-room",
                Some("A test"),
                None,
                Some("agent-1"),
            )
            .unwrap();
        assert_eq!(room.name, "test-room");
        assert_eq!(room.description, Some("A test".into()));

        let fetched = store.get_room("test-room").unwrap().unwrap();
        assert_eq!(fetched.room_id, "test-room");
    }

    #[test]
    fn test_duplicate_room_name() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_room("r1", "same-name", None, None, None)
            .unwrap();
        let result = store.create_room("r2", "same-name", None, None, None);
        assert!(matches!(result, Err(StoreError::RoomNameTaken(_))));
    }

    #[test]
    fn room_name_validation_is_canonical_for_creation() {
        let store = Store::open_in_memory().unwrap();
        let room = store
            .create_room("trimmed", "  Project Room  ", None, None, Some("creator"))
            .unwrap();
        assert_eq!(room.name, "Project Room");

        for (room_id, name) in [
            ("empty", "   ".to_string()),
            ("control", "bad\nname".to_string()),
            ("too-long", "🦀".repeat(MAX_ROOM_NAME_CHARS + 1)),
        ] {
            assert!(matches!(
                store.create_room(room_id, &name, None, None, Some("creator")),
                Err(StoreError::InvalidRoomName(_))
            ));
        }
        let longest_valid = "é".repeat(MAX_ROOM_NAME_CHARS);
        assert_eq!(
            store
                .create_room("longest-valid", &longest_valid, None, None, Some("creator"))
                .unwrap()
                .name,
            longest_valid
        );
    }

    #[test]
    fn rename_room_is_creator_and_key_authorized_persistent_and_unique() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_room_with_visibility(
                "target",
                "Old Name",
                None,
                None,
                Some("creator"),
                "private",
                Some("owner-key"),
                false,
            )
            .unwrap();
        store
            .create_room("conflict", "Existing Name", None, None, Some("other"))
            .unwrap();

        assert!(matches!(
            store.rename_room_authorized("target", "other", "owner-key", false, "New Name"),
            Err(RenameRoomError::AccessDenied)
        ));
        assert!(matches!(
            store.rename_room_authorized("target", "creator", "wrong-key", false, "New Name"),
            Err(RenameRoomError::AccessDenied)
        ));
        assert!(matches!(
            store.rename_room_authorized("target", "creator", "owner-key", false, "Existing Name"),
            Err(RenameRoomError::NameTaken(_))
        ));
        assert!(matches!(
            store.rename_room_authorized("lobby", "creator", "owner-key", false, "New Lobby"),
            Err(RenameRoomError::ProtectedRoom)
        ));
        assert!(matches!(
            store.rename_room_authorized("target", "creator", "owner-key", false, "\n"),
            Err(RenameRoomError::InvalidName(_))
        ));

        let renamed = store
            .rename_room_authorized("target", "creator", "owner-key", false, "  New Name  ")
            .unwrap();
        assert_eq!(renamed.name, "New Name");
        assert_eq!(store.get_room("target").unwrap().unwrap().name, "New Name");
    }

    #[test]
    fn destroy_room_is_creator_authorized_transactional_and_complete() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_room_with_visibility(
                "doomed",
                "doomed",
                None,
                None,
                Some("creator"),
                "private",
                Some("owner-key"),
                false,
            )
            .unwrap();
        store
            .insert_message(
                "message",
                "doomed",
                "creator",
                "Creator",
                "secret",
                None,
                &serde_json::json!({}),
            )
            .unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO votes
                 (vote_id, room_id, title, options, created_by, eligible_voters)
                 VALUES ('vote', 'doomed', 'Vote', '[\"yes\",\"no\"]', 'creator', 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO vote_eligible_agents (vote_id, agent_id)
                 VALUES ('vote', 'creator')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO vote_ballots (vote_id, agent_id, agent_name, option_index)
                 VALUES ('vote', 'creator', 'Creator', 0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO room_tasks
                 (task_id, room_id, title, status, created_by, created_at)
                 VALUES ('task', 'doomed', 'Task', 'pending', 'creator',
                         strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                [],
            )
            .unwrap();
        }
        store
            .create_subscription(
                "subscription",
                "doomed",
                "owner-key",
                "https://example.com/hook",
                "secret",
                &[],
                None,
                None,
                false,
                0,
            )
            .unwrap();
        store
            .enqueue_delivery("delivery", "subscription", 1, "message", chrono::Utc::now())
            .unwrap();

        assert!(matches!(
            store.destroy_room_authorized("doomed", "other-agent", "owner-key", false),
            Err(DestroyRoomError::AccessDenied)
        ));
        assert!(store.get_room("doomed").unwrap().is_some());
        assert!(matches!(
            store.destroy_room_authorized("lobby", "creator", "owner-key", false),
            Err(DestroyRoomError::ProtectedRoom)
        ));

        let destroyed = store
            .destroy_room_authorized("doomed", "creator", "owner-key", false)
            .unwrap();
        assert_eq!(destroyed.owner_key.as_deref(), Some("owner-key"));

        let conn = store.conn.lock().unwrap();
        for (table, predicate) in [
            ("rooms", "room_id = 'doomed'"),
            ("messages", "room_id = 'doomed'"),
            ("room_sequences", "room_id = 'doomed'"),
            ("votes", "room_id = 'doomed'"),
            ("vote_ballots", "vote_id = 'vote'"),
            ("vote_eligible_agents", "vote_id = 'vote'"),
            ("room_tasks", "room_id = 'doomed'"),
            ("subscriptions", "room_id = 'doomed'"),
            (
                "subscription_deliveries",
                "subscription_id = 'subscription'",
            ),
        ] {
            let count: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {predicate}"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "{table} must be cleaned");
        }
    }

    #[test]
    fn explicitly_keyless_rooms_use_a_durable_marker_and_remain_mutable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("explicit-keyless.db");
        let store = Store::open(&path).unwrap();
        store
            .create_room_with_visibility(
                "keyless-room",
                "Keyless Room",
                None,
                None,
                Some("creator"),
                "private",
                None,
                false,
            )
            .unwrap();

        let persisted_owner: Option<String> = store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT owner_key FROM rooms WHERE room_id = 'keyless-room'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(persisted_owner.as_deref(), Some(""));
        drop(store);

        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.get_room("keyless-room").unwrap().unwrap().owner_key,
            None
        );

        assert!(matches!(
            store.rename_room_authorized(
                "keyless-room",
                "creator",
                "authenticated-key",
                false,
                "Denied"
            ),
            Err(RenameRoomError::AccessDenied)
        ));
        store
            .rename_room_authorized("keyless-room", "creator", "", false, "Renamed")
            .unwrap();
        store
            .destroy_room_authorized("keyless-room", "creator", "", false)
            .unwrap();
    }

    #[test]
    fn legacy_null_owner_room_cannot_be_claimed_by_keyless_creator_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy-null-owner.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE rooms (
                    room_id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE,
                    description TEXT, parent_id TEXT, created_by TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                    visibility TEXT NOT NULL DEFAULT 'private', owner_key TEXT,
                    encrypted INTEGER NOT NULL DEFAULT 0
                 );
                 INSERT INTO rooms
                    (room_id, name, created_by, visibility, owner_key)
                 VALUES ('legacy-private', 'Legacy Private', 'spoofable-id', 'private', NULL);",
            )
            .unwrap();
        }

        let store = Store::open(&path).unwrap();
        let legacy = store.get_room("legacy-private").unwrap().unwrap();
        assert_eq!(legacy.owner_key.as_deref(), Some(LEGACY_UNOWNED_OWNER_KEY));
        assert!(matches!(
            store.rename_room_authorized("legacy-private", "spoofable-id", "", false, "Claimed"),
            Err(RenameRoomError::AccessDenied)
        ));
        assert!(matches!(
            store.destroy_room_authorized("legacy-private", "spoofable-id", "", false),
            Err(DestroyRoomError::AccessDenied)
        ));
        assert!(matches!(
            store.destroy_room_authorized("legacy-private", "spoofable-id", "", true),
            Err(DestroyRoomError::AccessDenied)
        ));
        assert!(store.get_room("legacy-private").unwrap().is_some());
    }

    #[test]
    fn public_visibility_does_not_grant_room_mutation_authority() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_room_with_visibility(
                "public-owned",
                "Public Owned",
                None,
                None,
                Some("creator"),
                "public",
                Some("owner-key"),
                false,
            )
            .unwrap();
        assert!(matches!(
            store.rename_room_authorized("public-owned", "creator", "other-key", false, "Denied"),
            Err(RenameRoomError::AccessDenied)
        ));
        assert!(matches!(
            store.destroy_room_authorized("public-owned", "creator", "other-key", false),
            Err(DestroyRoomError::AccessDenied)
        ));
    }

    #[test]
    fn destroy_nulls_child_parent_in_legacy_schema_without_foreign_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy-parent.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE rooms (
                    room_id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE,
                    description TEXT, parent_id TEXT, created_by TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                    visibility TEXT NOT NULL DEFAULT 'private', owner_key TEXT,
                    encrypted INTEGER NOT NULL DEFAULT 0
                 );",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        store
            .create_room_with_visibility(
                "parent",
                "Parent",
                None,
                None,
                Some("creator"),
                "private",
                Some("owner-key"),
                false,
            )
            .unwrap();
        store
            .create_room_with_visibility(
                "child",
                "Child",
                None,
                Some("parent"),
                Some("creator"),
                "private",
                Some("owner-key"),
                false,
            )
            .unwrap();
        store
            .destroy_room_authorized("parent", "creator", "owner-key", false)
            .unwrap();
        drop(store);

        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.get_room("child").unwrap().unwrap().parent_id, None);
    }

    #[test]
    fn test_insert_and_get_history() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_message(
                "msg-1",
                "lobby",
                "agent-1",
                "Alice",
                "Hello",
                None,
                &serde_json::json!({}),
            )
            .unwrap();
        store
            .insert_message(
                "msg-2",
                "lobby",
                "agent-2",
                "Bob",
                "Hi there",
                Some("msg-1"),
                &serde_json::json!({}),
            )
            .unwrap();

        let history = store.get_history("lobby", 50, None).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, "Hello");
        assert_eq!(history[1].content, "Hi there");
        assert_eq!(history[1].reply_to_message, Some("msg-1".into()));
        // seq should be 1, 2 in insertion order, per-room.
        assert_eq!(history[0].seq, 1);
        assert_eq!(history[1].seq, 2);
        assert_eq!(store.room_tip("lobby").unwrap(), 2);
    }

    #[test]
    fn test_seq_is_per_room() {
        let store = Store::open_in_memory().unwrap();
        store.create_room("r1", "r1", None, None, None).unwrap();
        store.create_room("r2", "r2", None, None, None).unwrap();
        let m1 = store
            .insert_message("a", "r1", "ag", "A", "hi", None, &serde_json::json!({}))
            .unwrap();
        let m2 = store
            .insert_message("b", "r2", "ag", "A", "hi", None, &serde_json::json!({}))
            .unwrap();
        let m3 = store
            .insert_message("c", "r1", "ag", "A", "hi2", None, &serde_json::json!({}))
            .unwrap();
        assert_eq!((m1.seq, m2.seq, m3.seq), (1, 1, 2));
        assert_eq!(store.room_tip("r1").unwrap(), 2);
        assert_eq!(store.room_tip("r2").unwrap(), 1);
        assert_eq!(store.room_tip("nonexistent").unwrap(), 0);
    }

    #[test]
    fn test_history_since_seq() {
        let store = Store::open_in_memory().unwrap();
        for i in 0..5 {
            store
                .insert_message(
                    &format!("m{i}"),
                    "lobby",
                    "ag",
                    "A",
                    &format!("msg{i}"),
                    None,
                    &serde_json::json!({}),
                )
                .unwrap();
        }
        let history = store
            .get_history_filtered("lobby", 50, None, None, Some(2))
            .unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].seq, 3);
        assert_eq!(history[2].seq, 5);
    }

    #[test]
    fn test_list_rooms_with_parent() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_room("parent", "parent-room", None, None, None)
            .unwrap();
        store
            .create_room("child-1", "child-1", None, Some("parent"), None)
            .unwrap();
        store
            .create_room("child-2", "child-2", None, Some("parent"), None)
            .unwrap();

        let children = store.list_rooms(Some("parent")).unwrap();
        assert_eq!(children.len(), 2);

        let all = store.list_rooms(None).unwrap();
        assert!(all.len() >= 4); // lobby + parent + 2 children
    }

    #[test]
    fn test_purge_messages_by_tier() {
        let store = Store::open_in_memory().unwrap();
        // A room with no owner key resolves to the 'free' tier via COALESCE.
        store
            .create_room("r1", "purge-room", None, None, None)
            .unwrap();
        store
            .insert_message(
                "m1",
                "r1",
                "a1",
                "agent",
                "hello",
                None,
                &serde_json::json!({}),
            )
            .unwrap();

        // A freshly-inserted message is inside the 14-day free window — kept.
        assert_eq!(store.purge_messages_by_tier("free", "-14 days").unwrap(), 0);
        // A pro sweep must not touch a free room, even with a future cutoff.
        assert_eq!(store.purge_messages_by_tier("pro", "+1 hours").unwrap(), 0);
        assert_eq!(store.get_history("r1", 10, None).unwrap().len(), 1);

        // Once the cutoff moves past the message, the free sweep deletes it.
        assert_eq!(store.purge_messages_by_tier("free", "+1 hours").unwrap(), 1);
        assert_eq!(store.get_history("r1", 10, None).unwrap().len(), 0);
    }

    #[test]
    fn test_full_purge_preserves_sequence_and_cursor_recovery() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_room("r1", "retention-room", None, None, None)
            .unwrap();
        for seq in 1..=2 {
            let message = store
                .insert_message(
                    &format!("m{seq}"),
                    "r1",
                    "agent",
                    "Agent",
                    "before purge",
                    None,
                    &serde_json::json!({}),
                )
                .unwrap();
            assert_eq!(message.seq, seq);
        }
        store
            .create_subscription(
                "sub-1",
                "r1",
                "owner",
                "https://example.com/hook",
                "secret",
                &[],
                None,
                None,
                false,
                2,
            )
            .unwrap();

        assert_eq!(store.purge_messages_by_tier("free", "+1 hours").unwrap(), 2);
        assert_eq!(store.get_history("r1", 10, None).unwrap().len(), 0);
        assert_eq!(store.room_tip("r1").unwrap(), 2);

        let after = store
            .insert_message(
                "m3",
                "r1",
                "agent",
                "Agent",
                "after purge",
                None,
                &serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(after.seq, 3);
        let resumed = store
            .get_history_filtered("r1", 10, None, None, Some(2))
            .unwrap();
        assert_eq!(resumed.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![3]);

        let subscription = store.get_subscription("sub-1").unwrap().unwrap().0;
        assert_eq!(subscription.last_delivered_seq, 2);
        assert!(store
            .enqueue_delivery(
                "delivery-3",
                "sub-1",
                after.seq,
                &after.message_id,
                chrono::Utc::now(),
            )
            .unwrap());
        let due = store.load_due_deliveries(chrono::Utc::now(), 10).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].message_seq, 3);
    }

    #[test]
    fn test_due_webhook_deliveries_are_serialized_per_subscription() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_subscription(
                "ordered-sub",
                "lobby",
                "owner",
                "https://example.com/hook",
                "secret",
                &[],
                None,
                None,
                false,
                0,
            )
            .unwrap();
        let now = chrono::Utc::now();
        store
            .enqueue_delivery("delivery-2", "ordered-sub", 2, "message-2", now)
            .unwrap();
        store
            .enqueue_delivery("delivery-1", "ordered-sub", 1, "message-1", now)
            .unwrap();
        let due = store.load_due_deliveries(now, 10).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].message_seq, 1);
        store.delete_delivery("delivery-1").unwrap();
        let next = store.load_due_deliveries(now, 10).unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].message_seq, 2);
    }

    #[test]
    fn test_sequence_high_water_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cowchat.db");
        {
            let store = Store::open(&path).unwrap();
            for seq in 1..=2 {
                let message = store
                    .insert_message(
                        &format!("m{seq}"),
                        "lobby",
                        "agent",
                        "Agent",
                        "before restart",
                        None,
                        &serde_json::json!({}),
                    )
                    .unwrap();
                assert_eq!(message.seq, seq);
            }
        }
        {
            let store = Store::open(&path).unwrap();
            assert_eq!(store.room_tip("lobby").unwrap(), 2);
            assert_eq!(store.purge_messages_by_tier("free", "+1 hours").unwrap(), 2);
            let message = store
                .insert_message(
                    "m3",
                    "lobby",
                    "agent",
                    "Agent",
                    "after restart and purge",
                    None,
                    &serde_json::json!({}),
                )
                .unwrap();
            assert_eq!(message.seq, 3);
        }
    }

    #[test]
    fn test_agent_identity_ownership_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.db");
        {
            let store = Store::open(&path).unwrap();
            assert!(store.claim_agent_identity("stable-agent", "key-a").unwrap());
        }
        let store = Store::open(&path).unwrap();
        assert!(store.claim_agent_identity("stable-agent", "key-a").unwrap());
        assert!(!store.claim_agent_identity("stable-agent", "key-b").unwrap());
    }

    #[test]
    fn test_legacy_reconstructed_vote_that_is_already_complete_closes_on_migration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy-vote.db");
        {
            let store = Store::open(&path).unwrap();
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO votes
                 (vote_id, room_id, title, options, created_by, status, eligible_voters)
                 VALUES ('legacy-vote', 'lobby', 'Legacy', '[\"yes\",\"no\"]',
                         'legacy-creator', 'open', 2)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO vote_ballots
                 (vote_id, agent_id, agent_name, option_index)
                 VALUES ('legacy-vote', 'legacy-creator', 'Creator', 0)",
                [],
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let vote = store.get_vote_meta("legacy-vote").unwrap().unwrap();
        assert_eq!(vote.eligible_voters, 1);
        assert_eq!(vote.status, "closed");
    }

    #[test]
    #[cfg(unix)]
    fn test_database_permissions_are_created_and_repaired_to_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("permissions.db");
        let store = Store::open(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        for sidecar in [
            std::path::PathBuf::from(format!("{}-wal", path.display())),
            std::path::PathBuf::from(format!("{}-shm", path.display())),
        ] {
            if sidecar.exists() {
                assert_eq!(
                    std::fs::metadata(sidecar).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
        drop(store);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        drop(Store::open(&path).unwrap());
        let repaired = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(repaired, 0o600);
    }

    #[test]
    fn test_old_style_insert_fails_after_full_purge_instead_of_regressing_seq() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_message(
                "before-purge",
                "lobby",
                "agent",
                "Agent",
                "before",
                None,
                &serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(store.purge_messages_by_tier("free", "+1 hours").unwrap(), 1);

        // Simulate the INSERT emitted by an older binary, which derives 1 from
        // MAX(messages) after retention deleted every row and does not advance
        // room_sequences first. The compatibility trigger must fail closed.
        let conn = store.conn.lock().unwrap();
        let result = conn.execute(
            "INSERT INTO messages
             (message_id, room_id, agent_id, agent_name, content, metadata, seq)
             VALUES ('old-binary', 'lobby', 'agent', 'Agent', 'unsafe', '{}', 1)",
            [],
        );
        assert!(result.is_err(), "a downgraded insert must not regress seq");
        drop(conn);
        assert_eq!(store.room_tip("lobby").unwrap(), 1);
    }

    #[test]
    fn test_concurrent_sequence_allocation_is_unique_and_monotonic() {
        let store = std::sync::Arc::new(Store::open_in_memory().unwrap());
        let workers = (0..24)
            .map(|index| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store
                        .insert_message(
                            &format!("concurrent-{index}"),
                            "lobby",
                            "agent",
                            "Agent",
                            "concurrent",
                            None,
                            &serde_json::json!({}),
                        )
                        .unwrap()
                        .seq
                })
            })
            .collect::<Vec<_>>();
        let mut sequences = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        sequences.sort_unstable();
        assert_eq!(sequences, (1..=24).collect::<Vec<_>>());
        assert_eq!(store.room_tip("lobby").unwrap(), 24);
    }

    #[test]
    fn test_single_use_invite_double_redeem_race_has_exactly_one_winner() {
        let store = std::sync::Arc::new(Store::open_in_memory().unwrap());
        store
            .create_room("invited", "invited", None, None, Some("creator"))
            .unwrap();
        store
            .create_invite("hash-single", "invited", "owner-key", true)
            .unwrap();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let winners = (0..8)
            .map(|_| {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.redeem_invite("hash-single").unwrap()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(winners, vec!["invited".to_string()]);

        let invite = store.get_invite("hash-single").unwrap().unwrap();
        assert!(invite.revoked);
        assert_eq!(invite.redeemed_count, 1);
    }

    #[test]
    fn test_open_invite_redeems_repeatedly_until_revoked() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_room("open-room", "open-room", None, None, Some("creator"))
            .unwrap();
        store
            .create_invite("hash-open", "open-room", "owner-key", false)
            .unwrap();

        assert_eq!(
            store.redeem_invite("hash-open").unwrap().as_deref(),
            Some("open-room")
        );
        assert_eq!(
            store.redeem_invite("hash-open").unwrap().as_deref(),
            Some("open-room")
        );
        assert!(store.revoke_invite("hash-open").unwrap());
        assert_eq!(store.redeem_invite("hash-open").unwrap(), None);
        assert_eq!(store.redeem_invite("unknown-hash").unwrap(), None);
        assert!(!store.revoke_invite("unknown-hash").unwrap());
        assert_eq!(
            store
                .get_invite("hash-open")
                .unwrap()
                .unwrap()
                .redeemed_count,
            2
        );
    }

    #[test]
    fn test_room_grants_are_durable_cached_and_destroyed_with_the_room() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("grants.db");
        {
            let store = Store::open(&path).unwrap();
            store
                .create_room_with_visibility(
                    "granted-room",
                    "Granted Room",
                    None,
                    None,
                    Some("creator"),
                    "private",
                    Some("owner-key"),
                    false,
                )
                .unwrap();
            store
                .create_invite("hash", "granted-room", "owner-key", false)
                .unwrap();
            store
                .add_room_grant("stranger-key", "granted-room")
                .unwrap();
            assert!(store.key_has_grant("stranger-key", "granted-room"));
            assert!(!store.key_has_grant("", "granted-room"));
            assert!(!store.key_has_grant("stranger-key", "lobby"));
        }

        // Cache is rehydrated from room_grants on reopen.
        let store = Store::open(&path).unwrap();
        assert!(store.key_has_grant("stranger-key", "granted-room"));
        assert_eq!(
            store.granted_keys_for_room("granted-room"),
            HashSet::from(["stranger-key".to_string()])
        );
        let visible = store
            .list_rooms_for_key(Some("stranger-key"), None)
            .unwrap();
        assert!(visible.iter().any(|room| room.room_id == "granted-room"));

        // Destroying the room deletes its invites and grants, durably and in cache.
        store
            .destroy_room_authorized("granted-room", "creator", "owner-key", false)
            .unwrap();
        assert!(!store.key_has_grant("stranger-key", "granted-room"));
        assert!(store.get_invite("hash").unwrap().is_none());
        let conn = store.conn.lock().unwrap();
        for table in ["room_invites", "room_grants"] {
            let count: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE room_id = 'granted-room'"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "{table} must be cleaned on destroy");
        }
    }

    #[test]
    fn test_sequence_migration_seeds_existing_maximum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE rooms (
                    room_id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE,
                    description TEXT, parent_id TEXT, created_by TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                    visibility TEXT NOT NULL DEFAULT 'private', owner_key TEXT,
                    encrypted INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE messages (
                    message_id TEXT PRIMARY KEY, room_id TEXT NOT NULL,
                    agent_id TEXT NOT NULL, agent_name TEXT NOT NULL,
                    content TEXT NOT NULL, reply_to_message TEXT, metadata TEXT DEFAULT '{}',
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                    seq INTEGER NOT NULL DEFAULT 0
                 );
                 INSERT INTO rooms (room_id, name, visibility) VALUES ('legacy', 'legacy', 'public');
                 INSERT INTO messages (message_id, room_id, agent_id, agent_name, content, seq)
                    VALUES ('old-7', 'legacy', 'a', 'A', 'old', 7),
                           ('old-9', 'legacy', 'a', 'A', 'old', 9);",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.room_tip("legacy").unwrap(), 9);
        let message = store
            .insert_message(
                "new-10",
                "legacy",
                "a",
                "A",
                "new",
                None,
                &serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(message.seq, 10);
    }
}
