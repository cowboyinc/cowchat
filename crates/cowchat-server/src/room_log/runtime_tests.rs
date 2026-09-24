//! Actual broker + CBFS nodes. These are runtime boundary tests; authenticated
//! public server handlers and finalized chain authority are separate gates.
#[path = "hosted_tests.rs"]
mod hosted_tests;
use super::*;
use crate::room_log::{
    intent::{Intent, IntentError, IntentJournal},
    runtime::{FencedWriter, OwnerRuntime, RuntimeError, RuntimeLimits, WorkerIncarnation},
    tests::{create, key_cutover, key_prepare, keyed_message, message},
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
    promote_session(fixture, writers).await.0
}
async fn promote_session(
    fixture: &Fixture,
    writers: &mut WriterRegistry,
) -> (FencedWriter, SigningKey, wire::StreamGrantV2) {
    let worker = WorkerIncarnation::fresh();
    let allocation = writers
        .claim(
            writers.epoch().unwrap(),
            worker.writer_id(),
            worker.claim_id(),
        )
        .await
        .unwrap();
    let config = config(fixture, &worker, allocation.epoch());
    let (holder, grant) = (config.holder.clone(), config.grant.clone());
    let log = fixture.connect_config(config).await.unwrap();
    (worker.bind(allocation, log).unwrap(), holder, grant)
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
    let expected = serde_json::to_value(&*runtime.state().unwrap()).unwrap();
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
        serde_json::to_value(&*runtime.state().unwrap()).unwrap(),
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

#[tokio::test]
async fn different_worker_recovers_preparation_without_local_journal() {
    let fixture = Fixture::new().await;
    let (control, volume) = Storage::new().await;
    let mut writers = control.initialize_writers(volume).await;
    let (storage, volume) = Storage::new().await;
    let first_host = private_directory();
    let second_host = private_directory();
    let mut runtime = recover(
        promote(&fixture, &mut writers).await,
        storage.initialize_archive(volume).await,
        &first_host.path().join("intents.sqlite"),
    )
    .await;
    let old = keyed_message("before", "one", 0);
    let pending = key_prepare("one", 1, Some([10; 32]));
    runtime
        .submit(vec![
            create("one", 0),
            create("two", 0),
            key_prepare("one", 0, None),
            key_cutover("one", 0, None),
            old.clone(),
            pending.clone(),
        ])
        .await
        .unwrap();
    let prepared = runtime
        .state()
        .unwrap()
        .room("one")
        .unwrap()
        .key_preparation
        .clone();
    drop(runtime);
    // A new worker has only the shared broker/archive, not host one's journal.
    let archive = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    let mut runtime = recover(
        promote(&fixture, &mut writers).await,
        archive,
        &second_host.path().join("intents.sqlite"),
    )
    .await;
    assert_eq!(
        runtime
            .state()
            .unwrap()
            .room("one")
            .unwrap()
            .key_preparation,
        prepared
    );
    let batch = runtime
        .submit(vec![
            pending,
            old,
            keyed_message("during", "one", 0),
            message("unaffected", "two"),
        ])
        .await
        .unwrap();
    assert!(matches!(
        batch.outcomes[0],
        Outcome::KeyEpochPrepared { .. }
    ));
    assert!(matches!(
        batch.outcomes[1],
        Outcome::MessageAppended { sequence: 1, .. }
    ));
    assert_eq!(
        batch.outcomes[2],
        Outcome::Rejected {
            reason: Rejection::KeyTransitionPending
        }
    );
    assert!(matches!(batch.outcomes[3], Outcome::MessageAppended { .. }));
    let batch = runtime
        .submit(vec![
            key_cutover("one", 1, Some([10; 32])),
            keyed_message("after", "one", 1),
        ])
        .await
        .unwrap();
    assert!(matches!(
        batch.outcomes[0],
        Outcome::KeyEpochCommitted { key_epoch: 1, .. }
    ));
    assert!(matches!(
        batch.outcomes[1],
        Outcome::MessageAppended { sequence: 2, .. }
    ));
    assert!(runtime
        .state()
        .unwrap()
        .room("one")
        .unwrap()
        .key_preparation
        .is_none());
}

#[tokio::test]
async fn key_cutover_waits_for_archive_and_recovers_same_transition_after_cancellation() {
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
    let old = keyed_message("before", "one", 0);
    runtime
        .submit(vec![
            create("one", 0),
            key_prepare("one", 0, None),
            key_cutover("one", 0, None),
            old.clone(),
            key_prepare("one", 1, Some([10; 32])),
        ])
        .await
        .unwrap();
    drop(runtime);
    let held = Arc::new(HoldCommit {
        inner: storage.registry.clone(),
        entered: tokio::sync::Notify::new(),
    });
    let archive = storage.archive(storage.reopen().await, held.clone()).await;
    let mut runtime = recover(promote(&fixture, &mut writers).await, archive, &path).await;
    let view = runtime.view();
    let cutover = key_cutover("one", 1, Some([10; 32]));
    {
        let submit = runtime.submit(vec![cutover.clone()]);
        tokio::pin!(submit);
        tokio::select! {
            result = &mut submit => panic!("cutover returned before archive commit: {}", result.is_ok()),
            _ = held.entered.notified() => {}
        }
        assert_eq!(
            view.read()
                .unwrap()
                .room("one")
                .unwrap()
                .key_state
                .as_ref()
                .unwrap()
                .key_epoch,
            0
        );
        // Dropping the uncertain write retires the complete projection.
    }
    assert!(matches!(view.read(), Err(RuntimeError::Retired)));
    drop(runtime);
    let archive = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    let mut runtime = recover(promote(&fixture, &mut writers).await, archive, &path).await;
    assert_eq!(
        runtime
            .state()
            .unwrap()
            .room("one")
            .unwrap()
            .key_state
            .as_ref()
            .unwrap()
            .key_epoch,
        1
    );
    let retried = runtime.submit(vec![cutover, old]).await.unwrap();
    assert!(retried.applied.is_empty());
    assert!(matches!(
        retried.outcomes[0],
        Outcome::KeyEpochCommitted { key_epoch: 1, .. }
    ));
    assert!(matches!(
        retried.outcomes[1],
        Outcome::MessageAppended { sequence: 1, .. }
    ));
    let batch = runtime
        .submit(vec![
            keyed_message("fresh-old", "one", 0),
            keyed_message("after", "one", 1),
        ])
        .await
        .unwrap();
    assert_eq!(
        batch.outcomes[0],
        Outcome::Rejected {
            reason: Rejection::KeyEpochMismatch
        }
    );
    assert!(matches!(
        batch.outcomes[1],
        Outcome::MessageAppended { sequence: 2, .. }
    ));
}

#[tokio::test]
async fn credential_horizon_advance_keeps_broker_writes_usable() {
    let fixture = Fixture::new().await;
    let (control, volume) = Storage::new().await;
    let mut writers = control.initialize_writers(volume).await;
    let (storage, volume) = Storage::new().await;
    let directory = private_directory();
    let mut runtime = recover(
        promote(&fixture, &mut writers).await,
        storage.initialize_archive(volume).await,
        &directory.path().join("intents.sqlite"),
    )
    .await;
    let original = std::time::Instant::now() + Duration::from_millis(50);
    runtime.advance_horizon(original).unwrap();
    runtime
        .advance_horizon(original + Duration::from_secs(30))
        .unwrap();
    tokio::time::sleep_until(original.into()).await;
    runtime
        .submit(vec![create("after-renewal", 0)])
        .await
        .unwrap();
    assert!(runtime
        .view()
        .read()
        .unwrap()
        .room("after-renewal")
        .is_some());
}

#[tokio::test]
async fn credential_deadline_retires_reads_and_prevents_new_broker_writes() {
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
    runtime.submit(vec![create("one", 0)]).await.unwrap();
    let view = runtime.view();
    assert!(runtime.advance_horizon(std::time::Instant::now()).is_err());
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
    runtime.advance_horizon(deadline).unwrap();
    assert!(runtime
        .advance_horizon(deadline - std::time::Duration::from_millis(1))
        .is_err());
    tokio::time::sleep_until(deadline.into()).await;
    assert!(matches!(view.read(), Err(RuntimeError::Retired)));
    assert!(matches!(runtime.state(), Err(RuntimeError::Retired)));
    assert!(matches!(
        runtime.submit(vec![message("a", "one")]).await,
        Err(RuntimeError::Retired)
    ));
    drop(runtime);
    let archive = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    let runtime = recover(promote(&fixture, &mut writers).await, archive, &path).await;
    assert_eq!(runtime.state().unwrap().applied_through(), 1);
    assert!(runtime
        .state()
        .unwrap()
        .room("one")
        .unwrap()
        .messages
        .is_empty());
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
    let view = runtime.view();
    {
        let submit = runtime.submit(vec![create("one", 0), message("a", "one")]);
        tokio::pin!(submit);
        tokio::select! {
            _ = held.entered.notified() => {},
            result = &mut submit => panic!("reply before held archive commit: {}", result.is_ok()),
            _ = tokio::time::sleep(DEADLINE) => panic!("archive commit not reached"),
        }
        assert!(view.read().unwrap().rooms().is_empty());
        // Dropping the future models a cancelled/disconnected request.
    }
    assert!(matches!(runtime.state(), Err(RuntimeError::Retired)));
    assert!(matches!(view.read(), Err(RuntimeError::Retired)));
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

#[cfg(feature = "hosted-bootstrap")]
mod renewal_run {
    use super::*;
    use crate::hosted_bootstrap::{Failure, Issue, Renewal};
    use std::time::Instant;

    #[derive(Clone, Copy)]
    enum Outcome {
        Renew,
        Retry,
        Terminal,
    }

    /// Re-signs the writer's own grant with a fresh nonce and lifetime, as the
    /// production issuer does, against the fixture broker.
    struct Reissue {
        fixture: Arc<Fixture>,
        holder: SigningKey,
        grant: wire::StreamGrantV2,
        outcome: Outcome,
        issued: u8,
    }

    impl Issue for Reissue {
        async fn attach(
            &mut self,
            now: u64,
            expiry: u64,
        ) -> Result<(SessionV2, wire::StreamGrantV2), Failure> {
            match self.outcome {
                Outcome::Retry => return Err(Failure::Retry(anyhow::anyhow!("broker down"))),
                Outcome::Terminal => return Err(Failure::Terminal(anyhow::anyhow!("revoked"))),
                Outcome::Renew => {}
            }
            self.issued += 1;
            let mut config = self.fixture.config(self.grant.policy_epoch);
            config.holder = self.holder.clone();
            config.grant = self.grant.clone();
            config.grant.grant_nonce = [self.issued; 32];
            config.grant.not_before_ms = now;
            config.grant.expires_at_ms = expiry;
            config.grant.signature = wire::CbqsSignatureV2(
                key(0xA1)
                    .sign(&cowboy_protocol_codec::keccak256(
                        &wire::stream_grant_signing_bytes_v2(&config.grant),
                    ))
                    .to_bytes(),
            );
            let grant = config.grant.clone();
            let socket = cbqs_client::connect_socket(&config.broker_url)
                .await
                .unwrap();
            let session = SessionV2::attach(socket, config, now_ms()).await.unwrap();
            Ok((session, grant))
        }
    }

    struct Rig {
        _fixture: Arc<Fixture>,
        _writers: WriterRegistry,
        writer: tokio::sync::Mutex<OwnerRuntime>,
        _storage: [Storage; 2],
        _directory: tempfile::TempDir,
    }

    /// A recovered runtime whose first grant leaves `horizon` of safe life,
    /// plus the renewal that owns it.
    async fn rig(horizon: Duration, outcome: Outcome) -> (Rig, Renewal<Reissue>) {
        let fixture = Arc::new(Fixture::new().await);
        let (control, volume) = Storage::new().await;
        let mut writers = control.initialize_writers(volume).await;
        let (storage, volume) = Storage::new().await;
        let directory = private_directory();
        let (writer, holder, grant) = promote_session(&fixture, &mut writers).await;
        let mut runtime = recover(
            writer,
            storage.initialize_archive(volume).await,
            &directory.path().join("intents.sqlite"),
        )
        .await;
        let now = now_ms();
        let renewal = Renewal::new(
            Reissue {
                fixture: fixture.clone(),
                holder,
                grant,
                outcome,
                issued: 0,
            },
            7_200_000,
            1,
            now + 30_000 + horizon.as_millis() as u64,
            now + 86_400_000 * 2,
            Vec::new(),
        )
        .unwrap();
        runtime.advance_horizon(renewal.horizon()).unwrap();
        let rig = Rig {
            _fixture: fixture,
            _writers: writers,
            writer: tokio::sync::Mutex::new(runtime),
            _storage: [control, storage],
            _directory: directory,
        };
        (rig, renewal)
    }

    #[tokio::test]
    async fn run_renews_past_the_original_horizon() {
        let (rig, renewal) = rig(Duration::from_secs(2), Outcome::Renew).await;
        let view = rig.writer.lock().await.view();
        let original = renewal.horizon();
        tokio::select! {
            result = renewal.run(&rig.writer, &view) => panic!("renewal ended: {result:?}"),
            _ = tokio::time::sleep_until((original + Duration::from_secs(1)).into()) => {}
        }
        let mut runtime = rig.writer.lock().await;
        runtime.submit(vec![create("renewed", 0)]).await.unwrap();
        assert!(view.read().unwrap().room("renewed").is_some());
    }

    #[tokio::test]
    async fn run_retires_at_the_horizon_when_retries_never_succeed() {
        let (rig, renewal) = rig(Duration::from_millis(1_500), Outcome::Retry).await;
        let view = rig.writer.lock().await.view();
        let horizon = renewal.horizon();
        assert!(renewal.run(&rig.writer, &view).await.is_err());
        assert!(Instant::now() >= horizon);
        assert!(matches!(view.read(), Err(RuntimeError::Retired)));
    }

    #[tokio::test]
    async fn run_retires_immediately_on_terminal_failure() {
        let (rig, renewal) = rig(Duration::from_secs(4), Outcome::Terminal).await;
        let view = rig.writer.lock().await.view();
        let horizon = renewal.horizon();
        assert!(renewal.run(&rig.writer, &view).await.is_err());
        assert!(Instant::now() < horizon);
        assert!(matches!(view.read(), Err(RuntimeError::Retired)));
    }
}
