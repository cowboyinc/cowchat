//! Two independent worker processes and SDK journals, real CBFS storage nodes,
//! and the production standalone Sled CAS exposed by a loopback test fixture.
//! This is a cross-process concurrency test, not a deployed multi-host claim.
use super::*;
use axum::{extract::State, http::StatusCode, routing::post, Json};
use serde::{Deserialize, Serialize};
use std::sync::{atomic::AtomicUsize, Mutex};

#[derive(Serialize, Deserialize)]
struct Commit {
    volume: VolumeId,
    previous: ManifestRoot,
    next: ManifestRoot,
    delta: i64,
    relays: Vec<RelayByteDelta>,
    added: Vec<TaggedShardRef>,
    removed: Vec<TaggedShardRef>,
}

struct Metadata {
    volume: VolumeId,
    authority: Arc<LocalAuthoritativeStore>,
    registry: Arc<LocalManifestRegistry>,
    barrier: tokio::sync::Barrier,
    calls: AtomicUsize,
    predecessors: Mutex<Vec<ManifestRoot>>,
}

async fn root(
    State(state): State<Arc<Metadata>>,
) -> Result<Json<Option<ManifestRoot>>, StatusCode> {
    state
        .authority
        .get_root(&state.volume)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn commit(
    State(state): State<Arc<Metadata>>,
    Json(commit): Json<Commit>,
) -> Result<Json<u64>, (StatusCode, String)> {
    if commit.volume != state.volume {
        return Err((StatusCode::BAD_REQUEST, "wrong fixture volume".into()));
    }
    state.predecessors.lock().unwrap().push(commit.previous);
    // Neither candidate may enter the real CAS until BOTH have built their
    // manifest and reached this boundary with the same initial predecessor.
    if state.calls.fetch_add(1, Ordering::SeqCst) < 2 {
        state.barrier.wait().await;
    }
    state
        .registry
        .commit_manifest_v2(
            &commit.volume,
            &commit.previous,
            &commit.next,
            commit.delta,
            commit.relays,
            commit.added,
            commit.removed,
            &[],
        )
        .await
        .map(Json)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct RemoteHooks {
    volume: VolumeId,
    url: String,
    client: reqwest::Client,
}
#[async_trait::async_trait]
impl AuthoritativeStore for RemoteHooks {
    async fn get_root(&self, volume: &VolumeId) -> Result<Option<ManifestRoot>, HookError> {
        if volume != &self.volume {
            return Err("wrong fixture volume".into());
        }
        Ok(self
            .client
            .get(format!("{}/root", self.url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
}
#[async_trait::async_trait]
impl ManifestRegistry for RemoteHooks {
    async fn is_shard_live(&self, _: &ShardId, _: u8) -> Result<bool, HookError> {
        Err("fixture does not implement GC liveness".into())
    }
    async fn commit_manifest_v2(
        &self,
        volume: &VolumeId,
        previous: &ManifestRoot,
        next: &ManifestRoot,
        delta: i64,
        relays: Vec<RelayByteDelta>,
        added: Vec<TaggedShardRef>,
        removed: Vec<TaggedShardRef>,
        token: &[u8],
    ) -> Result<u64, HookError> {
        if !token.is_empty() {
            return Err("fixture only accepts empty test tokens".into());
        }
        let body = Commit {
            volume: *volume,
            previous: *previous,
            next: *next,
            delta,
            relays,
            added,
            removed,
        };
        Ok(self
            .client
            .post(format!("{}/commit", self.url))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
}

#[derive(Serialize, Deserialize)]
struct WorkerInput {
    volume: VolumeId,
    wrapped_dek: Option<cbfs_types::WrappedDek>,
    nodes: Vec<NodeInfo>,
    metadata_url: String,
    writer_id: String,
    output: PathBuf,
}

/// Invoked twice by the parent test below. Explicitly ignored in normal runs;
/// it requires the parent-owned nodes, metadata server and input artifact.
#[tokio::test]
#[ignore = "subprocess entry point exercised by real_processes_race_one_promotion"]
async fn claim_process() {
    cbfs_sdk::test_support::isolate_state_dirs();
    let input = std::env::var_os("COWCHAT_TEST_CLAIM_INPUT").expect("parent input artifact");
    let input: WorkerInput = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    let hooks = Arc::new(RemoteHooks {
        volume: input.volume,
        url: input.metadata_url,
        client: reqwest::Client::builder()
            .no_proxy()
            .timeout(DEADLINE)
            .build()
            .unwrap(),
    });
    let root = hooks.get_root(&input.volume).await.unwrap().unwrap();
    let volume = Volume::open(
        input.volume,
        input.wrapped_dek.as_ref(),
        &WRAPPING_KEY,
        Visibility::Private,
        2,
        1,
        input.nodes,
        vec![],
        &root,
        Arc::new(QuicClient::new().unwrap()),
        Arc::new(RoundRobinSelector::new()),
    )
    .await
    .unwrap();
    let mut owners = WriterRegistry::open(volume, hooks.clone(), hooks, INSTANCE, STREAM, DEADLINE)
        .await
        .unwrap();
    let result = owners
        .claim(0, &input.writer_id, &format!("claim-{}", input.writer_id))
        .await;
    let output = match result {
        Ok(receipt) => serde_json::json!({
            "ok": true, "epoch": receipt.epoch(), "writer": receipt.writer_id(),
            "claim": receipt.claim_id(), "root": receipt.root(),
        }),
        Err(error) => serde_json::json!({"ok": false, "error": error.to_string()}),
    };
    std::fs::write(input.output, serde_json::to_vec(&output).unwrap()).unwrap();
}

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn real_processes_race_one_promotion() {
    let (storage, volume) = Storage::new().await;
    drop(storage.initialize_writers(volume).await);
    let initial_root = storage
        .authority
        .get_root(&storage.handle.volume_id)
        .await
        .unwrap()
        .unwrap();
    let metadata = Arc::new(Metadata {
        volume: storage.handle.volume_id,
        authority: storage.authority.clone(),
        registry: storage.registry.clone(),
        barrier: tokio::sync::Barrier::new(2),
        calls: AtomicUsize::new(0),
        predecessors: Mutex::new(Vec::new()),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/root", get(root))
        .route("/commit", post(commit))
        .with_state(metadata.clone());
    let _server = Server(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    let files = tempfile::tempdir().unwrap();
    let mut workers = Vec::new();
    for writer in ["a", "b"] {
        let input = WorkerInput {
            volume: storage.handle.volume_id,
            wrapped_dek: storage.handle.wrapped_dek.clone(),
            nodes: storage.nodes.iter().map(Node::info).collect(),
            metadata_url: url.clone(),
            writer_id: writer.into(),
            output: files.path().join(format!("{writer}.result")),
        };
        let path = files.path().join(format!("{writer}.json"));
        std::fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
        let log = std::fs::File::create(files.path().join(format!("{writer}.log"))).unwrap();
        workers.push((Worker(Process::new(std::env::current_exe().unwrap())
            .args(["--exact", "room_log::cbqs_tests::cbfs_archive_tests::ownership_process_tests::claim_process", "--ignored", "--nocapture"])
            .env_clear().env("COWCHAT_TEST_CLAIM_INPUT", &path)
            .stdin(Stdio::null()).stdout(log.try_clone().unwrap()).stderr(log)
            .spawn().unwrap()), writer));
    }
    for (worker, writer) in &mut workers {
        let status = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Some(status) = worker.0.try_wait().unwrap() {
                    break status;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("promotion worker deadline");
        assert!(
            status.success(),
            "worker failed: {}",
            std::fs::read_to_string(files.path().join(format!("{writer}.log"))).unwrap()
        );
    }
    assert_eq!(metadata.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        *metadata.predecessors.lock().unwrap(),
        vec![initial_root, initial_root]
    );
    let outcomes: Vec<serde_json::Value> = ["a", "b"]
        .iter()
        .map(|writer| {
            serde_json::from_slice(
                &std::fs::read(files.path().join(format!("{writer}.result"))).unwrap(),
            )
            .unwrap()
        })
        .collect();
    let winners: Vec<_> = outcomes.iter().filter(|v| v["ok"] == true).collect();
    assert_eq!(winners.len(), 1, "exactly one CAS winner: {outcomes:?}");
    assert_eq!(winners[0]["epoch"], 1);
    let mut cold = storage
        .writers(storage.reopen().await, storage.registry.clone())
        .await;
    let receipt = cold
        .claim(
            0,
            winners[0]["writer"].as_str().unwrap(),
            winners[0]["claim"].as_str().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(receipt.root()).unwrap(),
        winners[0]["root"]
    );
    assert_eq!(receipt.epoch(), 1);
}
