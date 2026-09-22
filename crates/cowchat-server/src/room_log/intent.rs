//! Local pending batches, not a room projection or cross-host recovery source.
//! SQLite supplies atomic durable replacement; a process lock prevents two
//! worker incarnations from consuming the same journal. Network I/O never runs
//! inside a SQLite transaction, and blocking disk work stays off Tokio threads.
use super::{Command, CommandBody};
use fs2::FileExt;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Debug, thiserror::Error)]
pub enum IntentError {
    #[error("intent journal is in use by another worker")]
    Locked,
    #[error("intent journal is unavailable, locked or corrupt")]
    Unavailable,
    #[error("intent journal belongs to a different owner stream")]
    Identity,
    #[error("pending intent batch exceeds configured bounds")]
    Limit,
    #[error("pending intent batch must be reconciled before replacement")]
    Pending,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_journal_unlocks_even_with_an_inherited_file_description() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let path = directory.path().join("intents.sqlite");
        let first =
            IntentJournal::open(&path, "owner-a".into(), [1; 32], [2; 32], 10, 1000).unwrap();
        // dup shares the same open-file description, as does fork before exec.
        let inherited = first.inner.lock().unwrap()._lock.0.try_clone().unwrap();
        drop(first);
        let next =
            IntentJournal::open(&path, "owner-a".into(), [1; 32], [2; 32], 10, 1000).unwrap();
        drop(next);
        drop(inherited);
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Intent {
    pub lane_id: u64,
    pub command: Command,
}

struct Inner {
    db: Connection,
    _lock: JournalLock,
}

struct JournalLock(File);
impl Drop for JournalLock {
    fn drop(&mut self) {
        // Explicit release also covers a concurrently spawned child's brief
        // fork-to-exec inheritance of this open file description.
        let _ = FileExt::unlock(&self.0);
    }
}

pub struct IntentJournal {
    inner: Arc<Mutex<Inner>>,
    owner_id: String,
    identity: ([u8; 32], [u8; 32]),
    max_records: usize,
    max_bytes: usize,
}

impl IntentJournal {
    /// Use an existing worker-private directory. Retain it for same-host
    /// recovery; a replacement on another host may start with an empty journal.
    /// Deleting this file cannot delete any acknowledged room history.
    pub fn open(
        path: &Path,
        owner_id: String,
        instance: [u8; 32],
        stream: [u8; 32],
        max_records: usize,
        max_bytes: usize,
    ) -> Result<Self, IntentError> {
        if !super::valid_identity(&owner_id) || max_records == 0 || max_bytes == 0 {
            return Err(IntentError::Limit);
        }
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            // Existing directories/files must also be private; do not silently
            // chmod unrelated caller files or follow a supplied symlink.
            let parent = path.parent().ok_or(IntentError::Unavailable)?;
            let meta = std::fs::symlink_metadata(parent).map_err(|_| IntentError::Unavailable)?;
            if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
                return Err(IntentError::Unavailable);
            }
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let lock_path = path.with_extension("lock");
        if lock_path == path {
            return Err(IntentError::Unavailable);
        }
        let lock = options
            .open(lock_path)
            .map_err(|_| IntentError::Unavailable)?;
        lock.try_lock_exclusive().map_err(|_| IntentError::Locked)?;
        let lock = JournalLock(lock);
        let file = options.open(path).map_err(|_| IntentError::Unavailable)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let metadata = file.metadata().map_err(|_| IntentError::Unavailable)?;
            if metadata.permissions().mode() & 0o077 != 0 || metadata.nlink() != 1 {
                return Err(IntentError::Unavailable);
            }
        }
        drop(file);
        let db = Connection::open(path).map_err(|_| IntentError::Unavailable)?;
        // EXTRA also syncs the directory after deleting the rollback journal.
        // fullfsync requests the platform's stronger flush where available.
        db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=EXTRA; PRAGMA fullfsync=ON;
            CREATE TABLE IF NOT EXISTS binding (id INTEGER PRIMARY KEY CHECK(id=1), value BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS pending (id INTEGER PRIMARY KEY CHECK(id=1), value BLOB NOT NULL);")
            .map_err(|_| IntentError::Unavailable)?;
        let binding = serde_json::to_vec(&(1u8, &owner_id, instance, stream))
            .map_err(|_| IntentError::Unavailable)?;
        db.execute(
            "INSERT OR IGNORE INTO binding VALUES (1, ?1)",
            params![&binding],
        )
        .map_err(|_| IntentError::Unavailable)?;
        let actual: Vec<u8> = db
            .query_row("SELECT value FROM binding WHERE id=1", [], |row| row.get(0))
            .map_err(|_| IntentError::Unavailable)?;
        if actual != binding {
            return Err(IntentError::Identity);
        }
        // Make initial database/name durable too, before any append may follow.
        File::open(path.parent().ok_or(IntentError::Unavailable)?)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| IntentError::Unavailable)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Inner { db, _lock: lock })),
            owner_id,
            identity: (instance, stream),
            max_records,
            max_bytes,
        })
    }

    pub(crate) fn matches(&self, owner: &str, identity: ([u8; 32], [u8; 32])) -> bool {
        self.owner_id == owner && self.identity == identity
    }

    fn encode(&self, intents: &[Intent]) -> Result<Vec<u8>, IntentError> {
        if intents.len() > self.max_records {
            return Err(IntentError::Limit);
        }
        if intents.iter().any(|i| i.command.owner_id != self.owner_id) {
            return Err(IntentError::Identity);
        }
        if intents.iter().any(|i| matches!(&i.command.body,
            CommandBody::AppendMessage {ciphertext, ..} if !cowchat_core::crypto::is_ciphertext(ciphertext))) {
            return Err(IntentError::Identity);
        }
        let bytes = serde_json::to_vec(&(1u8, intents)).map_err(|_| IntentError::Unavailable)?;
        if bytes.len() > self.max_bytes {
            return Err(IntentError::Limit);
        }
        Ok(bytes)
    }

    pub(crate) async fn pending(&self) -> Result<Vec<Intent>, IntentError> {
        let max_bytes = self.max_bytes;
        let bytes = self
            .disk(move |db| {
                let size: Option<usize> = db
                    .query_row("SELECT length(value) FROM pending WHERE id=1", [], |r| {
                        r.get(0)
                    })
                    .optional()
                    .map_err(|_| IntentError::Unavailable)?;
                if size.is_some_and(|n| n > max_bytes) {
                    return Err(IntentError::Limit);
                }
                db.query_row("SELECT value FROM pending WHERE id=1", [], |r| {
                    r.get::<_, Vec<u8>>(0)
                })
                .optional()
                .map_err(|_| IntentError::Unavailable)
            })
            .await?;
        let Some(bytes) = bytes else {
            return Ok(Vec::new());
        };
        let (version, intents): (u8, Vec<Intent>) =
            serde_json::from_slice(&bytes).map_err(|_| IntentError::Unavailable)?;
        if version != 1 {
            return Err(IntentError::Unavailable);
        }
        self.encode(&intents)?;
        Ok(intents)
    }

    pub(crate) async fn stage(&self, intents: &[Intent]) -> Result<(), IntentError> {
        let bytes = self.encode(intents)?;
        self.disk(move |db| {
            let exists: bool = db
                .query_row("SELECT EXISTS(SELECT 1 FROM pending)", [], |r| r.get(0))
                .map_err(|_| IntentError::Unavailable)?;
            if exists {
                return Err(IntentError::Pending);
            }
            db.execute("INSERT INTO pending VALUES (1, ?1)", params![bytes])
                .map_err(|_| IntentError::Unavailable)?;
            Ok(())
        })
        .await
    }

    pub(crate) async fn clear(&self) -> Result<(), IntentError> {
        self.disk(|db| {
            db.execute("DELETE FROM pending WHERE id=1", [])
                .map_err(|_| IntentError::Unavailable)?;
            Ok(())
        })
        .await
    }

    async fn disk<T: Send + 'static>(
        &self,
        op: impl FnOnce(&Connection) -> Result<T, IntentError> + Send + 'static,
    ) -> Result<T, IntentError> {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let guard = inner.lock().map_err(|_| IntentError::Unavailable)?;
            op(&guard.db)
        })
        .await
        .map_err(|_| IntentError::Unavailable)?
    }
}
