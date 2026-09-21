//! Production CBFS node processes + SDK, with the standalone Sled CAS authority.
//! This proves local shard recovery, not chain finality or cross-host failover.
#[path = "ownership_process_tests.rs"]
mod ownership_process_tests;
use super::*;
use crate::room_log::cbfs_archive::{ArchiveCommit, ArchiveError, CbfsArchive, RecoveryLimits};
use crate::room_log::ownership::{OwnershipError, WriterRegistry};
use cbfs_hooks::{
    standalone::{LocalAuthoritativeStore, LocalManifestRegistry, RoundRobinSelector},
    traits::{AuthoritativeStore, ManifestRegistry},
};
use cbfs_sdk::{volume::VolumeHandle, Volume};
use cbfs_transport::QuicClient;
use cbfs_types::{
    ManifestRoot, NodeId, NodeInfo, RelayByteDelta, ShardId, TaggedShardRef, Visibility,
    VolumeConfig, VolumeId,
};
use std::{
    path::PathBuf,
    process::{Child, Command as Process, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_BYTES: usize = 1_000_000;
const DEADLINE: Duration = Duration::from_secs(15);
const WRAPPING_KEY: [u8; 32] = [0x43; 32];
type HookError = Box<dyn std::error::Error + Send + Sync>;

struct Node {
    child: Option<Child>,
    binary: PathBuf,
    config: PathBuf,
    directory: tempfile::TempDir,
    addr: SocketAddr,
}
impl Drop for Node {
    fn drop(&mut self) {
        self.stop();
    }
}
impl Node {
    async fn new() -> Self {
        let binary = std::env::var_os("COWCHAT_TEST_CBFS_NODE")
            .map(PathBuf::from)
            .expect("build pinned cbfs-node and set COWCHAT_TEST_CBFS_NODE; no mock fallback");
        let directory = tempfile::tempdir().unwrap();
        let addr = std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let config = directory.path().join("node.toml");
        std::fs::write(
            &config,
            format!(
                "listen_addr = \"{addr}\"\ndata_dir = {}\ngc_interval_secs = 3600\nrepair_interval_secs = 3600\n",
                serde_json::to_string(&directory.path().join("data")).unwrap()
            ),
        )
        .unwrap();
        let mut node = Self {
            child: None,
            binary,
            config,
            directory,
            addr,
        };
        node.start().await;
        node
    }
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    async fn start(&mut self) {
        let output = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.directory.path().join("node.log"))
            .unwrap();
        self.child = Some(
            Process::new(&self.binary)
                .arg("--config")
                .arg(&self.config)
                .env_clear()
                // Explicit dev auth, accepted by the binary only on loopback.
                .env("CBFS_ACCEPT_ALL_AUTH", "1")
                .env("RUST_LOG", "warn")
                .stdin(Stdio::null())
                .stdout(output.try_clone().unwrap())
                .stderr(output)
                .spawn()
                .unwrap(),
        );
        let client =
            QuicClient::with_timeouts(Duration::from_secs(5), Duration::from_millis(200)).unwrap();
        tokio::time::timeout(DEADLINE, async {
            loop {
                assert!(
                    self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                    "cbfs-node exited: {}",
                    std::fs::read_to_string(self.directory.path().join("node.log")).unwrap()
                );
                if client.peer_cert_sha256(self.addr).await.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("cbfs-node readiness deadline");
    }
    fn info(&self) -> NodeInfo {
        NodeInfo {
            node_id: NodeId(
                std::fs::read(self.directory.path().join("data/node_id"))
                    .unwrap()
                    .try_into()
                    .unwrap(),
            ),
            addr: self.addr,
            capacity_bytes: 1 << 30,
            used_bytes: 0,
            region: None,
        }
    }
}

struct Storage {
    nodes: Vec<Node>,
    handle: VolumeHandle,
    authority: Arc<LocalAuthoritativeStore>,
    registry: Arc<LocalManifestRegistry>,
    _directory: tempfile::TempDir,
}
impl Storage {
    async fn new() -> (Self, Volume) {
        // Only state-directory isolation is used from SDK test_support.
        // All shard writes go through the real node's commit_durable handler.
        cbfs_sdk::test_support::isolate_state_dirs();
        let mut nodes = Vec::new();
        for _ in 0..3 {
            nodes.push(Node::new().await);
        }
        let (volume, handle) = Volume::create(
            VolumeConfig {
                erasure_k: 2,
                erasure_m: 1,
                visibility: Visibility::Private,
            },
            &WRAPPING_KEY,
            nodes.iter().map(Node::info).collect(),
            vec![],
            Arc::new(QuicClient::new().unwrap()),
            Arc::new(RoundRobinSelector::new()),
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let db = sled::open(directory.path()).unwrap();
        let roots = db.open_tree("manifest_roots").unwrap();
        roots
            .insert(handle.volume_id.as_ref(), ManifestRoot::default().as_ref())
            .unwrap();
        roots.flush().unwrap();
        (
            Self {
                nodes,
                handle,
                authority: Arc::new(LocalAuthoritativeStore::new(db.clone())),
                registry: Arc::new(LocalManifestRegistry::new(db)),
                _directory: directory,
            },
            volume,
        )
    }
    async fn reopen(&self) -> Volume {
        let root = self
            .authority
            .get_root(&self.handle.volume_id)
            .await
            .unwrap()
            .unwrap();
        Volume::open(
            self.handle.volume_id,
            self.handle.wrapped_dek.as_ref(),
            &WRAPPING_KEY,
            Visibility::Private,
            2,
            1,
            self.nodes.iter().map(Node::info).collect(),
            vec![],
            &root,
            Arc::new(QuicClient::new().unwrap()),
            Arc::new(RoundRobinSelector::new()),
        )
        .await
        .unwrap()
    }
    async fn archive(&self, volume: Volume, registry: Arc<dyn ManifestRegistry>) -> CbfsArchive {
        CbfsArchive::open(
            volume,
            self.authority.clone(),
            registry,
            INSTANCE,
            STREAM,
            MAX_BYTES,
            DEADLINE,
        )
        .await
        .unwrap()
    }
    async fn initialize_archive(&self, newly_created: Volume) -> CbfsArchive {
        CbfsArchive::initialize_new_volume(
            newly_created,
            self.authority.clone(),
            self.registry.clone(),
            INSTANCE,
            STREAM,
            MAX_BYTES,
            DEADLINE,
        )
        .await
        .unwrap()
    }
    async fn writers(&self, volume: Volume, registry: Arc<dyn ManifestRegistry>) -> WriterRegistry {
        WriterRegistry::open(
            volume,
            self.authority.clone(),
            registry,
            INSTANCE,
            STREAM,
            DEADLINE,
        )
        .await
        .unwrap()
    }
    async fn initialize_writers(&self, newly_created: Volume) -> WriterRegistry {
        WriterRegistry::initialize_new_volume(
            newly_created,
            self.authority.clone(),
            self.registry.clone(),
            INSTANCE,
            STREAM,
            DEADLINE,
        )
        .await
        .unwrap()
    }
}

struct LoseReply {
    inner: Arc<LocalManifestRegistry>,
    lost: AtomicBool,
}
#[async_trait::async_trait]
impl ManifestRegistry for LoseReply {
    async fn is_shard_live(&self, id: &ShardId, index: u8) -> Result<bool, HookError> {
        self.inner.is_shard_live(id, index).await
    }
    async fn commit_manifest_v2(
        &self,
        id: &VolumeId,
        prev: &ManifestRoot,
        root: &ManifestRoot,
        delta: i64,
        relays: Vec<RelayByteDelta>,
        added: Vec<TaggedShardRef>,
        removed: Vec<TaggedShardRef>,
        token: &[u8],
    ) -> Result<u64, HookError> {
        let seq = self
            .inner
            .commit_manifest_v2(id, prev, root, delta, relays, added, removed, token)
            .await?;
        self.lost.store(true, Ordering::SeqCst);
        Err(format!("injected lost reply after durable CAS sequence {seq}").into())
    }
}

async fn batch(fixture: &Fixture) -> (CbqsOwnerLog, super::super::cbqs::Replay, u64) {
    let mut log = fixture.connect(1).await.unwrap();
    let lane = log.room_lane("owner-a", "one").await.unwrap();
    log.append(0, &super::super::tests::create("one", lane))
        .await
        .unwrap();
    log.append(lane, &super::super::tests::message("a", "one"))
        .await
        .unwrap();
    let replay = log.replay(None, 2, 10, MAX_BYTES).await.unwrap();
    (log, replay, lane)
}

fn recovery_limits() -> RecoveryLimits {
    RecoveryLimits {
        max_segments: 10,
        max_records: 10,
        max_bytes: MAX_BYTES,
    }
}

async fn three_batches(
    fixture: &Fixture,
    storage: &Storage,
    volume: Volume,
) -> (CbfsArchive, CbqsOwnerLog, Vec<ArchiveCommit>, u64) {
    let (mut log, mut replay, lane) = batch(fixture).await;
    let mut archive = storage.initialize_archive(volume).await;
    let first = archive.publish(&replay).await.unwrap();
    let repeat = archive.publish(&replay).await.unwrap();
    assert_eq!(repeat.head, first.head);
    assert_eq!(repeat.manifest_root, first.manifest_root);
    let mut commits = vec![first];
    for id in ["b", "c"] {
        let sequence = log
            .append(lane, &super::super::tests::message(id, "one"))
            .await
            .unwrap();
        replay = log
            .replay(replay.checkpoint.as_ref(), sequence, 10, MAX_BYTES)
            .await
            .unwrap();
        commits.push(archive.publish(&replay).await.unwrap());
    }
    (archive, log, commits, lane)
}

#[tokio::test]
async fn real_nodes_archive_repeat_restart_and_restore_after_broker_expiry() {
    let fixture = Fixture::new().await;
    let (mut storage, volume) = Storage::new().await;
    let (archive, log, commits, lane) = three_batches(&fixture, &storage, volume).await;
    let expected_head = commits.last().unwrap().head.clone();
    drop(archive);
    drop(log);
    drop(commits);
    fixture
        .core
        .store
        .advance_retention_floor(&STREAM, 5)
        .unwrap();
    // SIGKILL every relay, then reuse only their disk directories. Drop the
    // complete writer/SDK state and use new QUIC connections for recovery.
    for node in &mut storage.nodes {
        node.stop();
    }
    for node in &mut storage.nodes {
        node.start().await;
    }
    let mut recovered = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    assert_eq!(recovered.head().unwrap(), Some(&expected_head));
    let mut reader = fixture.connect(2).await.unwrap();
    let restored = recovered
        .recover(&mut reader, recovery_limits())
        .await
        .unwrap();
    assert_eq!(
        restored.checkpoint.as_ref().unwrap().sequence(),
        expected_head.sequence
    );
    let mut state = OwnerState::new("owner-a".into());
    for record in restored.records {
        state
            .apply(record.sequence, record.lane_id, &record.command)
            .unwrap();
    }
    assert_eq!(state.applied_through(), 4);
    assert_eq!(state.room("one").unwrap().messages.len(), 3);
    reader
        .append(lane, &super::super::tests::message("live", "one"))
        .await
        .unwrap();
    let tail = reader
        .replay(restored.checkpoint.as_ref(), 5, 1, MAX_BYTES)
        .await
        .unwrap();
    assert_eq!(tail.records.len(), 1);
    assert_eq!(tail.records[0].command.command_id, "live");
}

#[tokio::test]
async fn real_nodes_cold_archive_recovery_enforces_total_budgets() {
    let fixture = Fixture::new().await;
    let (storage, volume) = Storage::new().await;
    let (archive, _, commits, _) = three_batches(&fixture, &storage, volume).await;
    let mut total_bytes = 0;
    for commit in &commits {
        total_bytes += archive
            .read_segment(&commit.head.checkpoint)
            .await
            .unwrap()
            .len();
    }
    drop(archive);
    let limits = [
        RecoveryLimits {
            max_segments: 2,
            ..recovery_limits()
        },
        RecoveryLimits {
            max_records: 3,
            ..recovery_limits()
        },
        RecoveryLimits {
            max_bytes: total_bytes - 1,
            ..recovery_limits()
        },
    ];
    for (index, limit) in limits.into_iter().enumerate() {
        let mut archive = storage
            .archive(storage.reopen().await, storage.registry.clone())
            .await;
        let mut log = fixture.connect(index as u64 + 2).await.unwrap();
        assert!(matches!(
            archive.recover(&mut log, limit).await,
            Err(ArchiveError::Limit)
        ));
        assert!(matches!(archive.head(), Err(ArchiveError::Unavailable)));
    }
    let mut archive = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    let mut log = fixture.connect(5).await.unwrap();
    let exact = RecoveryLimits {
        max_segments: 3,
        max_records: 4,
        max_bytes: total_bytes,
    };
    assert_eq!(
        archive
            .recover(&mut log, exact)
            .await
            .unwrap()
            .records
            .len(),
        4
    );
}

#[tokio::test]
async fn real_nodes_cold_recovery_refuses_tampered_and_missing_middle_segments() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let fixture = Fixture::new().await;
    let (storage, volume) = Storage::new().await;
    let (archive, _, commits, _) = three_batches(&fixture, &storage, volume).await;
    let middle = &commits[1].head.checkpoint;
    let bytes = archive.read_segment(middle).await.unwrap();
    drop(archive);
    let path = format!(
        "cowchat/{}/{}/{}.json",
        &hex0x(&INSTANCE)[2..],
        &hex0x(&STREAM)[2..],
        &hex0x(middle)[2..]
    );
    let mut altered: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let payload = altered["records"][0]["payload"].as_str().unwrap();
    let mut command: serde_json::Value =
        serde_json::from_slice(&STANDARD.decode(payload).unwrap()).unwrap();
    command["body"]["ciphertext"] = "cow1:tampered".into();
    altered["records"][0]["payload"] = STANDARD
        .encode(serde_json::to_vec(&command).unwrap())
        .into();
    let mut volume = storage.reopen().await;
    volume
        .put(&path, &serde_json::to_vec(&altered).unwrap())
        .await
        .unwrap();
    volume
        .commit(storage.authority.as_ref(), storage.registry.as_ref())
        .await
        .unwrap();
    let mut archive = storage.archive(volume, storage.registry.clone()).await;
    let mut log = fixture.connect(2).await.unwrap();
    assert!(matches!(
        archive.recover(&mut log, recovery_limits()).await,
        Err(ArchiveError::Verification)
    ));
    assert!(matches!(archive.head(), Err(ArchiveError::Unavailable)));
    drop(archive);
    // Removing the middle object leaves a perfectly readable latest head and
    // final segment. Recovery must not silently return just that suffix.
    let mut volume = storage.reopen().await;
    assert!(volume.remove_path(&path).unwrap().is_some());
    volume
        .commit(storage.authority.as_ref(), storage.registry.as_ref())
        .await
        .unwrap();
    let mut archive = storage.archive(volume, storage.registry.clone()).await;
    let mut log = fixture.connect(3).await.unwrap();
    assert!(matches!(
        archive.recover(&mut log, recovery_limits()).await,
        Err(ArchiveError::HistoryGap)
    ));
    assert!(matches!(archive.head(), Err(ArchiveError::Unavailable)));
}

#[tokio::test]
async fn real_nodes_lost_commit_reply_reconciles_without_republishing() {
    let fixture = Fixture::new().await;
    let (mut log, replay, lane) = batch(&fixture).await;
    let (storage, volume) = Storage::new().await;
    let faulty = Arc::new(LoseReply {
        inner: storage.registry.clone(),
        lost: AtomicBool::new(false),
    });
    drop(storage.initialize_archive(volume).await);
    let mut archive = storage
        .archive(storage.reopen().await, faulty.clone())
        .await;
    assert!(matches!(
        archive.publish(&replay).await,
        Err(ArchiveError::Unavailable)
    ));
    assert!(
        faulty.lost.load(Ordering::SeqCst),
        "fault must occur after the actual CAS"
    );
    assert!(matches!(archive.head(), Err(ArchiveError::Unavailable)));
    assert!(matches!(
        archive.publish(&replay).await,
        Err(ArchiveError::Unavailable)
    ));
    let committed = storage
        .authority
        .get_root(&storage.handle.volume_id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(committed, ManifestRoot::default());
    drop(archive);
    let mut recovered = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    assert_eq!(recovered.head().unwrap().unwrap().sequence, 2);
    let receipt = recovered.publish(&replay).await.unwrap();
    assert_eq!(receipt.manifest_root, committed);
    // The old pending journal must not satisfy the NEXT batch's publication.
    log.append(lane, &super::super::tests::message("b", "one"))
        .await
        .unwrap();
    let suffix = log
        .replay(replay.checkpoint.as_ref(), 3, 10, MAX_BYTES)
        .await
        .unwrap();
    let next = recovered.publish(&suffix).await.unwrap();
    assert_ne!(next.manifest_root, committed);
    drop(recovered);
    let cold = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    assert_eq!(cold.head().unwrap().unwrap().sequence, 3);
    let bytes = cold.read_segment(&next.head.checkpoint).await.unwrap();
    let restored = log
        .restore_archive(&bytes, replay.checkpoint.as_ref(), 10, MAX_BYTES)
        .unwrap();
    assert_eq!(restored.records[0].command.command_id, "b");
}

#[tokio::test]
async fn real_nodes_stale_writer_and_backward_head_are_refused() {
    let fixture = Fixture::new().await;
    let (mut log, replay, lane) = batch(&fixture).await;
    let (storage, volume) = Storage::new().await;
    let mut current = storage.initialize_archive(volume).await;
    current.publish(&replay).await.unwrap();
    let mut stale = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    log.append(lane, &super::super::tests::message("b", "one"))
        .await
        .unwrap();
    let suffix = log
        .replay(replay.checkpoint.as_ref(), 3, 10, MAX_BYTES)
        .await
        .unwrap();
    let latest = current.publish(&suffix).await.unwrap();
    assert!(matches!(
        stale.publish(&suffix).await,
        Err(ArchiveError::Conflict)
    ));
    assert!(matches!(
        current.publish(&replay).await,
        Err(ArchiveError::Conflict)
    ));
    let recovered = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    assert_eq!(recovered.head().unwrap(), Some(&latest.head));
    assert_eq!(latest.head.sequence, 3);
}

#[tokio::test]
async fn real_nodes_promotion_retry_and_abandoned_epoch_takeover() {
    let fixture = Fixture::new().await;
    let (storage, volume) = Storage::new().await;
    let mut owners = storage.initialize_writers(volume).await;
    assert_eq!(owners.epoch().unwrap(), 0);
    let abandoned = owners.claim(0, "writer-a", "claim-a").await.unwrap();
    assert_eq!(abandoned.epoch(), 1);
    assert_eq!(abandoned.writer_id(), "writer-a");
    assert_eq!(abandoned.claim_id(), "claim-a");
    assert_eq!(abandoned.stream_identity(), (INSTANCE, STREAM));
    assert_eq!(abandoned.control_volume(), storage.handle.volume_id);
    let retry = owners.claim(0, "writer-a", "claim-a").await.unwrap();
    assert_eq!(retry.root(), abandoned.root());
    // The first claimant disappears BEFORE it ever attaches/fences CBQS.
    drop(owners);
    let mut stale = storage
        .writers(storage.reopen().await, storage.registry.clone())
        .await;
    let mut next = storage
        .writers(storage.reopen().await, storage.registry.clone())
        .await;
    let active = next.claim(1, "writer-b", "claim-b").await.unwrap();
    assert_eq!(active.epoch(), 2);
    assert!(matches!(
        stale.claim(1, "writer-c", "claim-c").await,
        Err(OwnershipError::Conflict)
    ));
    assert!(matches!(stale.epoch(), Err(OwnershipError::Unavailable)));
    // Claim receipts cannot append; only a matching fenced session can.
    let mut log = fixture.connect(active.epoch()).await.unwrap();
    assert!(matches!(
        fixture.connect(abandoned.epoch()).await,
        Err(LogError::Fenced)
    ));
    let lane = log.room_lane("owner-a", "one").await.unwrap();
    assert_eq!(
        log.append(0, &super::super::tests::create("one", lane))
            .await
            .unwrap(),
        1
    );
    let mut cold = storage
        .writers(storage.reopen().await, storage.registry.clone())
        .await;
    assert_eq!(cold.epoch().unwrap(), 2);
    assert!(matches!(
        cold.claim(1, "writer-c", "claim-b").await,
        Err(OwnershipError::Conflict)
    ));
}

#[tokio::test]
async fn real_nodes_lost_promotion_reply_reconciles_same_claim() {
    let (storage, volume) = Storage::new().await;
    let faulty = Arc::new(LoseReply {
        inner: storage.registry.clone(),
        lost: AtomicBool::new(false),
    });
    drop(storage.initialize_writers(volume).await);
    let mut owners = storage
        .writers(storage.reopen().await, faulty.clone())
        .await;
    assert!(matches!(
        owners.claim(0, "writer-a", "claim-a").await,
        Err(OwnershipError::Unavailable)
    ));
    assert!(faulty.lost.load(Ordering::SeqCst));
    assert!(matches!(
        owners.claim(0, "writer-a", "claim-a").await,
        Err(OwnershipError::Unavailable)
    ));
    drop(owners);
    let root = storage
        .authority
        .get_root(&storage.handle.volume_id)
        .await
        .unwrap()
        .unwrap();
    let mut cold = storage
        .writers(storage.reopen().await, storage.registry.clone())
        .await;
    let retry = cold.claim(0, "writer-a", "claim-a").await.unwrap();
    assert_eq!(retry.epoch(), 1);
    assert_eq!(retry.root(), root);
    assert!(matches!(
        cold.claim(0, "writer-b", "claim-b").await,
        Err(OwnershipError::Conflict)
    ));
}

#[tokio::test]
async fn real_nodes_missing_used_control_record_cannot_recreate_epoch_one() {
    let (storage, volume) = Storage::new().await;
    let mut owners = storage.initialize_writers(volume).await;
    owners.claim(0, "writer-a", "claim-a").await.unwrap();
    drop(owners);
    let mut volume = storage.reopen().await;
    let path = format!(
        "cowchat-owners/{}/{}.json",
        &hex0x(&INSTANCE)[2..],
        &hex0x(&STREAM)[2..]
    );
    assert!(volume.remove_path(&path).unwrap().is_some());
    volume
        .commit(storage.authority.as_ref(), storage.registry.as_ref())
        .await
        .unwrap();
    let cold = storage.reopen().await;
    assert!(matches!(
        WriterRegistry::open(
            cold,
            storage.authority.clone(),
            storage.registry.clone(),
            INSTANCE,
            STREAM,
            DEADLINE,
        )
        .await,
        Err(OwnershipError::InvalidRecord)
    ));
}

#[tokio::test]
async fn real_nodes_deleted_archive_cannot_reopen_as_empty_history() {
    let fixture = Fixture::new().await;
    let (_, replay, _) = batch(&fixture).await;
    let (storage, volume) = Storage::new().await;
    let mut archive = storage.initialize_archive(volume).await;
    archive.publish(&replay).await.unwrap();
    drop(archive);
    let mut volume = storage.reopen().await;
    for object in volume.list("") {
        assert!(volume.remove_path(&object.path).unwrap().is_some());
    }
    volume
        .commit(storage.authority.as_ref(), storage.registry.as_ref())
        .await
        .unwrap();
    assert_eq!(
        storage
            .authority
            .get_root(&storage.handle.volume_id)
            .await
            .unwrap(),
        Some(ManifestRoot::default())
    );
    let cold = storage.reopen().await;
    assert!(matches!(
        CbfsArchive::open(
            cold,
            storage.authority.clone(),
            storage.registry.clone(),
            INSTANCE,
            STREAM,
            MAX_BYTES,
            DEADLINE,
        )
        .await,
        Err(ArchiveError::HistoryGap)
    ));
}
