//! Actual broker + CBFS nodes. These are runtime boundary tests; authenticated
//! public server handlers and finalized chain authority are separate gates.
use super::*;
use crate::room_log::{
    intent::{Intent, IntentError, IntentJournal},
    runtime::{FencedWriter, OwnerRuntime, RuntimeError, RuntimeLimits, WorkerIncarnation},
    tests::{create, message},
};
use std::path::Path;

fn private_directory() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}
fn limits() -> RuntimeLimits {
    RuntimeLimits {
        archive: recovery_limits(),
        batch_records: 10,
        replay_bytes: MAX_BYTES,
    }
}
fn journal(path: &Path) -> IntentJournal {
    IntentJournal::open(path, "owner-a".into(), INSTANCE, STREAM, 10, MAX_BYTES).unwrap()
}
fn config(fixture: &Fixture, worker: &WorkerIncarnation, epoch: u64) -> SessionConfig {
    let mut config = fixture.config(epoch);
    config.holder = worker.holder().clone();
    config.grant.holder_signing_key.key_bytes = config.holder.verifying_key().to_bytes();
    config.grant.signature = wire::CbqsSignatureV2(
        key(0xA1)
            .sign(&cowboy_protocol_codec::keccak256(
                &wire::stream_grant_signing_bytes_v2(&config.grant),
            ))
            .to_bytes(),
    );
    config
}
async fn promote(fixture: &Fixture, writers: &mut WriterRegistry) -> FencedWriter {
    let worker = WorkerIncarnation::fresh();
    let allocation = writers
        .claim(
            writers.epoch().unwrap(),
            worker.writer_id(),
            worker.claim_id(),
        )
        .await
        .unwrap();
    let log = fixture
        .connect_config(config(fixture, &worker, allocation.epoch()))
        .await
        .unwrap();
    worker.bind(allocation, log).unwrap()
}
async fn recover(writer: FencedWriter, archive: CbfsArchive, path: &Path) -> OwnerRuntime {
    OwnerRuntime::recover("owner-a".into(), writer, archive, journal(path), limits())
        .await
        .unwrap()
}

#[tokio::test]
async fn durable_batch_retries_and_cold_projection_recovery() {
    let fixture = Fixture::new().await;
    let (control, volume) = Storage::new().await;
    let mut writers = control.initialize_writers(volume).await;
    let (storage, volume) = Storage::new().await;
    let directory = private_directory();
    let path = directory.path().join("intents.sqlite");
    let mut runtime = recover(
        promote(&fixture, &mut writers).await,
        storage.initialize_archive(volume).await,
        &path,
    )
    .await;
    let result = runtime
        .submit(vec![
            create("one", 0),
            create("two", 0),
            message("a", "one"),
            message("b", "two"),
        ])
        .await
        .unwrap();
    assert_eq!(result.applied.len(), 4);
    let first_receipt = result.outcomes[2].clone();
    let tip = runtime.state().unwrap().applied_through();
    let mut retry = message("a", "one");
    retry.timestamp = Utc::now();
    if let CommandBody::AppendMessage { agent_name, .. } = &mut retry.body {
        *agent_name = "Renamed".into();
    }
    let result = runtime.submit(vec![retry]).await.unwrap();
    assert_eq!(result.outcomes, vec![first_receipt]);
    assert!(result.applied.is_empty());
    assert_eq!(runtime.state().unwrap().applied_through(), tip);
    let mut conflict = message("a", "one");
    if let CommandBody::AppendMessage { ciphertext, .. } = &mut conflict.body {
        *ciphertext = "cow1:changed".into();
    }
    let result = runtime.submit(vec![conflict]).await.unwrap();
    assert_eq!(
        result.outcomes,
        vec![Outcome::Rejected {
            reason: Rejection::CommandConflict
        }]
    );
    assert!(result.applied.is_empty());
    let expected = serde_json::to_value(runtime.state().unwrap()).unwrap();
    drop(runtime);
    // Fresh host has no intents or projection, only the shared archive/log.
    let another_directory = private_directory();
    let archive = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    let runtime = recover(
        promote(&fixture, &mut writers).await,
        archive,
        &another_directory.path().join("intents.sqlite"),
    )
    .await;
    assert_eq!(
        serde_json::to_value(runtime.state().unwrap()).unwrap(),
        expected
    );
    assert_eq!(
        runtime.state().unwrap().room("one").unwrap().messages[0]
            .message
            .agent_name,
        "Alice"
    );
}

struct HoldCommit {
    inner: Arc<LocalManifestRegistry>,
    entered: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl ManifestRegistry for HoldCommit {
    async fn is_shard_live(&self, id: &ShardId, index: u8) -> Result<bool, HookError> {
        self.inner.is_shard_live(id, index).await
    }
    async fn commit_manifest_v2(
        &self,
        _id: &VolumeId,
        _prev: &ManifestRoot,
        _root: &ManifestRoot,
        _delta: i64,
        _relays: Vec<RelayByteDelta>,
        _added: Vec<TaggedShardRef>,
        _removed: Vec<TaggedShardRef>,
        _token: &[u8],
    ) -> Result<u64, HookError> {
        self.entered.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn cancelled_archive_withholds_reply_and_retires_reads_then_recovers_exact_intents() {
    let fixture = Fixture::new().await;
    let (control, volume) = Storage::new().await;
    let mut writers = control.initialize_writers(volume).await;
    let (storage, volume) = Storage::new().await;
    drop(storage.initialize_archive(volume).await);
    let held = Arc::new(HoldCommit {
        inner: storage.registry.clone(),
        entered: tokio::sync::Notify::new(),
    });
    let archive = storage.archive(storage.reopen().await, held.clone()).await;
    let initial_root = storage
        .authority
        .get_root(&storage.handle.volume_id)
        .await
        .unwrap();
    let directory = private_directory();
    let path = directory.path().join("intents.sqlite");
    let mut runtime = recover(promote(&fixture, &mut writers).await, archive, &path).await;
    {
        let submit = runtime.submit(vec![create("one", 0), message("a", "one")]);
        tokio::pin!(submit);
        tokio::select! {
            _ = held.entered.notified() => {},
            result = &mut submit => panic!("reply before held archive commit: {}", result.is_ok()),
            _ = tokio::time::sleep(DEADLINE) => panic!("archive commit not reached"),
        }
        // Dropping the future models a cancelled/disconnected request.
    }
    assert!(matches!(runtime.state(), Err(RuntimeError::Retired)));
    assert!(matches!(
        runtime.submit(vec![message("b", "one")]).await,
        Err(RuntimeError::Retired)
    ));
    drop(runtime);
    let pending = journal(&path);
    assert_eq!(pending.pending().await.unwrap().len(), 2);
    drop(pending);
    assert_eq!(
        storage
            .authority
            .get_root(&storage.handle.volume_id)
            .await
            .unwrap(),
        initial_root
    );
    // Reopening may complete the SDK's pending manifest commit. Either way,
    // recovery must derive the same two records without resending.
    let archive = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    let mut runtime = recover(promote(&fixture, &mut writers).await, archive, &path).await;
    assert_eq!(runtime.state().unwrap().applied_through(), 2);
    assert_eq!(
        runtime.state().unwrap().room("one").unwrap().messages.len(),
        1
    );
    let result = runtime.submit(vec![message("a", "one")]).await.unwrap();
    assert!(result.applied.is_empty());
    assert_eq!(runtime.state().unwrap().applied_through(), 2);
    drop(runtime);
    assert!(journal(&path).pending().await.unwrap().is_empty());
}

#[tokio::test]
async fn lost_archive_reply_recovers_without_duplicate_append() {
    let fixture = Fixture::new().await;
    let (control, volume) = Storage::new().await;
    let mut writers = control.initialize_writers(volume).await;
    let (storage, volume) = Storage::new().await;
    drop(storage.initialize_archive(volume).await);
    let lost = Arc::new(LoseReply {
        inner: storage.registry.clone(),
        lost: AtomicBool::new(false),
    });
    let archive = storage.archive(storage.reopen().await, lost.clone()).await;
    let directory = private_directory();
    let path = directory.path().join("intents.sqlite");
    let mut runtime = recover(promote(&fixture, &mut writers).await, archive, &path).await;
    assert!(runtime
        .submit(vec![create("one", 0), message("a", "one")])
        .await
        .is_err());
    assert!(lost.lost.load(Ordering::SeqCst));
    assert!(matches!(runtime.state(), Err(RuntimeError::Retired)));
    drop(runtime);
    let archive = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    let mut runtime = recover(promote(&fixture, &mut writers).await, archive, &path).await;
    assert_eq!(runtime.state().unwrap().applied_through(), 2);
    assert!(runtime
        .submit(vec![message("a", "one")])
        .await
        .unwrap()
        .applied
        .is_empty());
    assert_eq!(runtime.state().unwrap().applied_through(), 2);
}

#[tokio::test]
async fn partial_broker_batch_reconciles_then_appends_only_missing_commands() {
    let fixture = Fixture::new().await;
    let (control, volume) = Storage::new().await;
    let mut writers = control.initialize_writers(volume).await;
    let worker = WorkerIncarnation::fresh();
    let allocation = writers
        .claim(0, worker.writer_id(), worker.claim_id())
        .await
        .unwrap();
    let mut log = fixture
        .connect_config(config(&fixture, &worker, allocation.epoch()))
        .await
        .unwrap();
    let lane = log.room_lane("owner-a", "one").await.unwrap();
    let directory = private_directory();
    let path = directory.path().join("intents.sqlite");
    let pending = journal(&path);
    pending
        .stage(&[
            Intent {
                lane_id: 0,
                command: create("one", lane),
            },
            Intent {
                lane_id: lane,
                command: message("a", "one"),
            },
        ])
        .await
        .unwrap();
    log.append(0, &create("one", lane)).await.unwrap();
    drop(log);
    drop(pending);
    let (storage, volume) = Storage::new().await;
    let runtime = recover(
        promote(&fixture, &mut writers).await,
        storage.initialize_archive(volume).await,
        &path,
    )
    .await;
    assert_eq!(runtime.state().unwrap().applied_through(), 2);
    assert_eq!(
        runtime.state().unwrap().room("one").unwrap().messages.len(),
        1
    );
}

#[tokio::test]
async fn new_incarnation_cannot_reuse_another_claim_or_holder() {
    let fixture = Fixture::new().await;
    let (control, volume) = Storage::new().await;
    let mut writers = control.initialize_writers(volume).await;
    let first = WorkerIncarnation::fresh();
    let second = WorkerIncarnation::fresh();
    assert_ne!(first.writer_id(), second.writer_id());
    assert_ne!(first.claim_id(), second.claim_id());
    assert_ne!(
        first.holder().verifying_key(),
        second.holder().verifying_key()
    );
    let allocation = writers
        .claim(0, first.writer_id(), first.claim_id())
        .await
        .unwrap();
    let log = fixture
        .connect_config(config(&fixture, &second, allocation.epoch()))
        .await
        .unwrap();
    assert!(matches!(
        second.bind(allocation, log),
        Err(RuntimeError::Configuration)
    ));
    let allocation = writers
        .claim(0, first.writer_id(), first.claim_id())
        .await
        .unwrap();
    let log = fixture.connect(1).await.unwrap(); // independently authorized WRONG holder
    assert!(matches!(
        first.bind(allocation, log),
        Err(RuntimeError::Configuration)
    ));
}

#[tokio::test]
async fn journal_lock_binding_bounds_and_exact_pending_bytes_survive_reopen() {
    let directory = private_directory();
    let path = directory.path().join("intents.sqlite");
    let first = journal(&path);
    assert!(matches!(
        IntentJournal::open(&path, "owner-a".into(), INSTANCE, STREAM, 10, MAX_BYTES),
        Err(IntentError::Locked)
    ));
    let command = message("stable", "one");
    first
        .stage(&[Intent {
            lane_id: 7,
            command: command.clone(),
        }])
        .await
        .unwrap();
    assert!(matches!(first.stage(&[]).await, Err(IntentError::Pending)));
    drop(first);
    assert!(matches!(
        IntentJournal::open(&path, "owner-b".into(), INSTANCE, STREAM, 10, MAX_BYTES),
        Err(IntentError::Identity)
    ));
    let reopened = journal(&path);
    assert_eq!(
        serde_json::to_vec(&reopened.pending().await.unwrap()[0].command).unwrap(),
        serde_json::to_vec(&command).unwrap()
    );
    reopened.clear().await.unwrap();
    drop(reopened);
    let bounded = IntentJournal::open(&path, "owner-a".into(), INSTANCE, STREAM, 1, 10).unwrap();
    assert!(matches!(
        bounded
            .stage(&[Intent {
                lane_id: 7,
                command
            }])
            .await,
        Err(IntentError::Limit)
    ));
    assert!(bounded.pending().await.unwrap().is_empty());
}
