use super::*;
use crate::artifact::{self, Artifact, ArtifactError, PatchArtifact, PublishArtifact};
use rusqlite::{Transaction, TransactionBehavior};

impl From<rusqlite::Error> for ArtifactError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(StoreError::Db(error))
    }
}
impl From<serde_json::Error> for ArtifactError {
    fn from(error: serde_json::Error) -> Self {
        Self::Storage(StoreError::Json(error))
    }
}

impl Store {
    pub(super) fn initialize_artifacts(&self) -> Result<(), rusqlite::Error> {
        self.conn.lock().unwrap().execute_batch(
            "CREATE TABLE IF NOT EXISTS room_artifacts (
                room_id TEXT PRIMARY KEY REFERENCES rooms(room_id) ON DELETE CASCADE,
                public_id TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL, view TEXT NOT NULL, data TEXT NOT NULL,
                revision INTEGER NOT NULL CHECK(revision > 0), updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS room_artifact_receipts (
                room_id TEXT NOT NULL REFERENCES rooms(room_id) ON DELETE CASCADE,
                request_id TEXT NOT NULL, digest TEXT NOT NULL, author_hash TEXT NOT NULL,
                revision INTEGER NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY(room_id, request_id)
            );
            CREATE INDEX IF NOT EXISTS artifact_receipts_age ON room_artifact_receipts(created_at);
            CREATE INDEX IF NOT EXISTS artifact_receipts_author ON room_artifact_receipts(author_hash,created_at);
            CREATE TABLE IF NOT EXISTS room_artifact_write_limits (
                author_hash TEXT PRIMARY KEY, window_start INTEGER NOT NULL, count INTEGER NOT NULL
            );"
        )
    }

    pub fn get_artifact(&self, room_id: &str) -> Result<Option<Artifact>, ArtifactError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row("SELECT room_id,title,view,data,revision,updated_at,public_id FROM room_artifacts WHERE room_id=?1", [room_id], map_artifact).optional()?)
    }

    pub fn get_authorized_artifact(
        &self,
        room: &str,
        key: &str,
        no_auth: bool,
    ) -> Result<Option<Artifact>, ArtifactError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        authorize_artifact_room(&tx, room, key, no_auth, ArtifactAccess::Read)?;
        Ok(tx.query_row("SELECT room_id,title,view,data,revision,updated_at,public_id FROM room_artifacts WHERE room_id=?1", [room], map_artifact).optional()?)
    }

    pub fn get_public_artifact(&self, public_id: &str) -> Result<Option<Artifact>, ArtifactError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row("SELECT room_id,title,view,data,revision,updated_at,public_id FROM room_artifacts WHERE public_id=?1", [public_id], map_artifact).optional()?)
    }

    pub fn publish_artifact(
        &self,
        room: &str,
        key: &str,
        no_auth: bool,
        request: &PublishArtifact,
    ) -> Result<Artifact, ArtifactError> {
        artifact::validate_request_id(&request.request_id)?;
        artifact::validate_document(
            &request.title,
            &request.view,
            request.data.as_ref().unwrap_or(&serde_json::json!({})),
        )?;
        self.mutate_artifact(
            room,
            key,
            no_auth,
            &request.request_id,
            &serde_json::to_string(&("publish", request))?,
            |current| {
                if let Some(current) = &current {
                    if request.expected_revision != Some(current.revision) {
                        return Err(ArtifactError::Conflict);
                    }
                } else if request.expected_revision.is_some() {
                    return Err(ArtifactError::Conflict);
                }
                Ok(Artifact {
                    room_id: room.into(),
                    title: request.title.clone(),
                    view: request.view.clone(),
                    data: request
                        .data
                        .clone()
                        .or_else(|| current.as_ref().map(|a| a.data.clone()))
                        .unwrap_or_else(|| serde_json::json!({})),
                    revision: current.as_ref().map_or(1, |a| a.revision),
                    updated_at: current
                        .as_ref()
                        .map_or_else(|| Utc::now().to_rfc3339(), |a| a.updated_at.clone()),
                    public_id: current
                        .map_or_else(|| uuid::Uuid::new_v4().to_string(), |a| a.public_id),
                })
            },
        )
    }

    pub fn patch_artifact(
        &self,
        room: &str,
        key: &str,
        no_auth: bool,
        request: &PatchArtifact,
    ) -> Result<Artifact, ArtifactError> {
        artifact::validate_request_id(&request.request_id)?;
        artifact::validate_data(&request.patch)?;
        self.mutate_artifact(
            room,
            key,
            no_auth,
            &request.request_id,
            &serde_json::to_string(&("patch", request))?,
            |current| {
                let mut current = current.ok_or(ArtifactError::NotFound)?;
                if request
                    .expected_revision
                    .is_some_and(|r| r != current.revision)
                {
                    return Err(ArtifactError::Conflict);
                }
                artifact::merge_patch(&mut current.data, &request.patch);
                Ok(current)
            },
        )
    }

    pub fn rotate_artifact(
        &self,
        room: &str,
        key: &str,
        no_auth: bool,
        request_id: &str,
    ) -> Result<Artifact, ArtifactError> {
        artifact::validate_request_id(request_id)?;
        self.mutate_artifact(room, key, no_auth, request_id, "rotate", |current| {
            let mut current = current.ok_or(ArtifactError::NotFound)?;
            current.public_id = uuid::Uuid::new_v4().to_string();
            Ok(current)
        })
    }

    pub fn unpublish_artifact(
        &self,
        room: &str,
        key: &str,
        no_auth: bool,
    ) -> Result<(), ArtifactError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize_artifact_room(&tx, room, key, no_auth, ArtifactAccess::Write)?;
        take_artifact_write_slot(&tx, key)?;
        tx.execute("DELETE FROM room_artifacts WHERE room_id=?1", [room])?;
        tx.commit()?;
        drop(conn);
        self.artifact_changes.send_replace(());
        Ok(())
    }

    fn mutate_artifact(
        &self,
        room: &str,
        key: &str,
        no_auth: bool,
        request_id: &str,
        input: &str,
        mutate: impl FnOnce(Option<Artifact>) -> Result<Artifact, ArtifactError>,
    ) -> Result<Artifact, ArtifactError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize_artifact_room(&tx, room, key, no_auth, ArtifactAccess::Write)?;
        // Cleanup is bounded by the indexed retention timestamp. Receipts never replay writes.
        tx.execute(
            "DELETE FROM room_artifact_receipts WHERE created_at < unixepoch()-604800",
            [],
        )?;
        let digest = hex::encode(Sha256::digest(input.as_bytes()));
        let author_hash = hex::encode(Sha256::digest(key.as_bytes()));
        if let Some((prior_digest, prior_author)) = tx.query_row(
            "SELECT digest,author_hash FROM room_artifact_receipts WHERE room_id=?1 AND request_id=?2",
            params![room,request_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)),
        ).optional()? {
            if prior_digest != digest || prior_author != author_hash { return Err(ArtifactError::Conflict); }
            return tx.query_row("SELECT room_id,title,view,data,revision,updated_at,public_id FROM room_artifacts WHERE room_id=?1", [room], map_artifact).optional()?.ok_or(ArtifactError::NotFound);
        }
        take_artifact_write_slot(&tx, key)?;
        let current = tx.query_row("SELECT room_id,title,view,data,revision,updated_at,public_id FROM room_artifacts WHERE room_id=?1", [room], map_artifact).optional()?;
        let mut next = mutate(current.clone())?;
        artifact::validate_document(&next.title, &next.view, &next.data)?;
        let changed = current.as_ref() != Some(&next);
        if changed {
            next.revision = current.as_ref().map_or(1, |a| a.revision + 1);
            next.updated_at = Utc::now().to_rfc3339();
            write_artifact(&tx, &next)?;
        }
        tx.execute("INSERT INTO room_artifact_receipts(room_id,request_id,digest,author_hash,revision) VALUES(?1,?2,?3,?4,?5)", params![room,request_id,digest,author_hash,next.revision])?;
        tx.commit()?;
        drop(conn);
        if changed {
            self.artifact_changes.send_replace(());
        }
        Ok(next)
    }
}

fn map_artifact(row: &rusqlite::Row<'_>) -> rusqlite::Result<Artifact> {
    let data: String = row.get(3)?;
    Ok(Artifact {
        room_id: row.get(0)?,
        title: row.get(1)?,
        view: row.get(2)?,
        data: serde_json::from_str(&data).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?,
        revision: row.get(4)?,
        updated_at: row.get(5)?,
        public_id: row.get(6)?,
    })
}
fn write_artifact(tx: &Transaction<'_>, artifact: &Artifact) -> Result<(), ArtifactError> {
    tx.execute("INSERT INTO room_artifacts(room_id,title,view,data,revision,updated_at,public_id) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(room_id) DO UPDATE SET title=excluded.title, view=excluded.view, data=excluded.data, revision=excluded.revision, updated_at=excluded.updated_at, public_id=excluded.public_id", params![artifact.room_id,artifact.title,artifact.view,serde_json::to_string(&artifact.data)?,artifact.revision,artifact.updated_at,artifact.public_id])?;
    Ok(())
}

enum ArtifactAccess {
    Read,
    Write,
}

fn authorize_artifact_room(
    tx: &Transaction<'_>,
    room: &str,
    key: &str,
    no_auth: bool,
    access: ArtifactAccess,
) -> Result<(), ArtifactError> {
    let room = query_room_by_id(tx, room)?.ok_or(ArtifactError::NotFound)?;
    if let Some(allowed) = super::social::room_access(tx, &room.room_id, key)? {
        return if allowed {
            Ok(())
        } else {
            Err(ArtifactError::NotFound)
        };
    }
    let grant = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM room_grants WHERE room_id=?1 AND api_key=?2)",
        params![room.room_id, key],
        |r| r.get::<_, bool>(0),
    )?;
    let allowed = match access {
        ArtifactAccess::Read => crate::connection::can_access_room_parts(
            &room.visibility,
            room.owner_key.as_deref(),
            key,
            no_auth,
        ),
        ArtifactAccess::Write => {
            crate::connection::matches_room_owner(room.owner_key.as_deref(), key, no_auth)
        }
    };
    if allowed || grant {
        Ok(())
    } else {
        Err(ArtifactError::NotFound)
    }
}

fn take_artifact_write_slot(tx: &Transaction<'_>, key: &str) -> Result<(), ArtifactError> {
    let hash = hex::encode(Sha256::digest(key.as_bytes()));
    tx.execute("INSERT INTO room_artifact_write_limits(author_hash,window_start,count) VALUES(?1,unixepoch(),1) ON CONFLICT(author_hash) DO UPDATE SET count=CASE WHEN window_start <= unixepoch()-60 THEN 1 ELSE count+1 END, window_start=CASE WHEN window_start <= unixepoch()-60 THEN unixepoch() ELSE window_start END",[&hash])?;
    let count: i64 = tx.query_row(
        "SELECT count FROM room_artifact_write_limits WHERE author_hash=?1",
        [hash],
        |r| r.get(0),
    )?;
    if count > 120 {
        Err(ArtifactError::RateLimited)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn room(store: &Store, id: &str) {
        store
            .create_room_with_visibility(
                id,
                id,
                None,
                None,
                Some("author"),
                "private",
                Some("owner"),
                false,
            )
            .unwrap();
    }
    fn publish() -> PublishArtifact {
        PublishArtifact {
            title: "Board".into(),
            view: "root = Stack([])".into(),
            data: Some(serde_json::json!({"items":{"a":{"status":"open"}},"labels":[1,2]})),
            expected_revision: None,
            request_id: uuid::Uuid::new_v4().to_string(),
        }
    }
    fn patch(value: serde_json::Value, revision: Option<u64>) -> PatchArtifact {
        PatchArtifact {
            patch: value,
            expected_revision: revision,
            request_id: uuid::Uuid::new_v4().to_string(),
        }
    }
    #[test]
    fn artifact_merge_conflict_dedup_rotation_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifacts.db");
        let store = Store::open(&path).unwrap();
        room(&store, "target");
        room(&store, "other");
        let publication = publish();
        let first = store
            .publish_artifact("target", "owner", false, &publication)
            .unwrap();
        assert_eq!(first.revision, 1);
        assert!(store.get_artifact("other").unwrap().is_none());
        let update = patch(
            serde_json::json!({"items":{"b":{"status":"queued"}},"labels":null}),
            Some(1),
        );
        let second = store
            .patch_artifact("target", "owner", false, &update)
            .unwrap();
        assert_eq!(second.revision, 2);
        assert_eq!(second.data["items"]["a"]["status"], "open");
        assert!(second.data.get("labels").is_none());
        assert_eq!(
            store
                .patch_artifact("target", "owner", false, &update)
                .unwrap(),
            second
        );
        assert_eq!(
            store
                .publish_artifact("target", "owner", false, &publication)
                .unwrap(),
            second
        );
        assert!(matches!(
            store.patch_artifact(
                "target",
                "owner",
                false,
                &patch(serde_json::json!({"x":1}), Some(1))
            ),
            Err(ArtifactError::Conflict)
        ));
        let noop = store
            .patch_artifact(
                "target",
                "owner",
                false,
                &patch(serde_json::json!({}), None),
            )
            .unwrap();
        assert_eq!(noop.revision, 2);
        let mut replacement = publish();
        replacement.expected_revision = Some(2);
        replacement.data = None;
        let unchanged = store
            .publish_artifact("target", "owner", false, &replacement)
            .unwrap();
        assert_eq!(unchanged.revision, 2);
        assert_eq!(unchanged.data, second.data);
        let mut collision = update.clone();
        collision.patch = serde_json::json!({"x":4});
        assert!(matches!(
            store.patch_artifact("target", "owner", false, &collision),
            Err(ArtifactError::Conflict)
        ));
        let rotation = uuid::Uuid::new_v4().to_string();
        let rotated = store
            .rotate_artifact("target", "owner", false, &rotation)
            .unwrap();
        assert!(store
            .get_public_artifact(&first.public_id)
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .rotate_artifact("target", "owner", false, &rotation)
                .unwrap(),
            rotated
        );
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(
            reopened
                .get_public_artifact(&rotated.public_id)
                .unwrap()
                .unwrap(),
            rotated
        );
        assert_eq!(
            reopened
                .patch_artifact("target", "owner", false, &update)
                .unwrap(),
            rotated
        );
        reopened
            .unpublish_artifact("target", "owner", false)
            .unwrap();
        assert!(matches!(
            reopened.publish_artifact("target", "owner", false, &publication),
            Err(ArtifactError::NotFound)
        ));
        assert!(reopened
            .get_public_artifact(&rotated.public_id)
            .unwrap()
            .is_none());
    }
    #[test]
    fn artifact_parallel_writers_preserve_unrelated_keys_and_room_deletion_cascades() {
        let store = std::sync::Arc::new(Store::open_in_memory().unwrap());
        room(&store, "target");
        let artifact = store
            .publish_artifact("target", "owner", false, &publish())
            .unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let workers: Vec<_> = ["one", "two"]
            .into_iter()
            .map(|key| {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store
                        .patch_artifact(
                            "target",
                            "owner",
                            false,
                            &patch(serde_json::json!({"items":{key:{"done":true}}}), None),
                        )
                        .unwrap();
                })
            })
            .collect();
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        let result = store.get_artifact("target").unwrap().unwrap();
        assert_eq!(result.data["items"]["one"]["done"], true);
        assert_eq!(result.data["items"]["two"]["done"], true);
        assert_eq!(result.revision, 3);
        store
            .destroy_room_authorized("target", "author", "owner", false)
            .unwrap();
        assert!(store
            .get_public_artifact(&artifact.public_id)
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .conn
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM room_artifact_receipts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn artifact_rejects_private_room_access_and_limits_retry_retention() {
        let store = Store::open_in_memory().unwrap();
        room(&store, "target");
        assert!(matches!(
            store.publish_artifact("target", "wrong", false, &publish()),
            Err(ArtifactError::NotFound)
        ));
        assert!(matches!(
            store.publish_artifact("missing", "owner", false, &publish()),
            Err(ArtifactError::NotFound)
        ));
        let publication = publish();
        store
            .publish_artifact("target", "owner", false, &publication)
            .unwrap();
        for _ in 0..119 {
            store
                .patch_artifact(
                    "target",
                    "owner",
                    false,
                    &patch(serde_json::json!({}), None),
                )
                .unwrap();
        }
        assert!(matches!(
            store.patch_artifact(
                "target",
                "owner",
                false,
                &patch(serde_json::json!({"x":1}), None)
            ),
            Err(ArtifactError::RateLimited)
        ));
        assert!(store
            .publish_artifact("target", "owner", false, &publication)
            .is_ok());
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE room_artifact_receipts SET created_at=unixepoch()-604801",
                [],
            )
            .unwrap();
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE room_artifact_write_limits SET window_start=unixepoch()-61",
                [],
            )
            .unwrap();
        assert!(store
            .patch_artifact(
                "target",
                "owner",
                false,
                &patch(serde_json::json!({"x":1}), None)
            )
            .is_ok());
        assert_eq!(
            store
                .conn
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM room_artifact_receipts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
