//! Production CBFS node processes + SDK, with the standalone Sled CAS authority.
//! This proves local shard recovery, not chain finality or cross-host failover.
use super::*;
use crate::room_log::cbfs_archive::{ArchiveError, CbfsArchive};
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

#[tokio::test]
async fn real_nodes_archive_repeat_restart_and_restore_after_broker_expiry() {
    let fixture = Fixture::new().await;
    let (log, replay, _) = batch(&fixture).await;
    let (mut storage, volume) = Storage::new().await;
    let mut archive = storage.archive(volume, storage.registry.clone()).await;
    let first = archive.publish(&replay).await.unwrap();
    assert_eq!(first.head.sequence, 2);
    let repeat = archive.publish(&replay).await.unwrap();
    assert_eq!(repeat.head, first.head);
    assert_eq!(repeat.manifest_root, first.manifest_root);
    drop(archive);
    drop(log);
    drop(replay);
    fixture
        .core
        .store
        .advance_retention_floor(&STREAM, 3)
        .unwrap();
    // SIGKILL every relay, then reuse only their disk directories. Drop the
    // complete writer/SDK state and use new QUIC connections for recovery.
    for node in &mut storage.nodes {
        node.stop();
    }
    for node in &mut storage.nodes {
        node.start().await;
    }
    let recovered = storage
        .archive(storage.reopen().await, storage.registry.clone())
        .await;
    let head = recovered.head().unwrap().unwrap();
    assert_eq!(head, &first.head);
    let bytes = recovered.read_segment(&head.checkpoint).await.unwrap();
    let mut reader = fixture.connect(2).await.unwrap();
    let restored = reader.restore_archive(&bytes, None, 10, MAX_BYTES).unwrap();
    assert_eq!(
        restored.checkpoint.as_ref().unwrap().sequence(),
        head.sequence
    );
    let mut state = OwnerState::new("owner-a".into());
    for record in restored.records {
        state
            .apply(record.sequence, record.lane_id, &record.command)
            .unwrap();
    }
    assert_eq!(state.room("one").unwrap().messages.len(), 1);
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
    let mut archive = storage.archive(volume, faulty.clone()).await;
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
    let mut current = storage.archive(volume, storage.registry.clone()).await;
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
