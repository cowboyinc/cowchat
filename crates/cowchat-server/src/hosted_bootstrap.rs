//! Explicit production hosted bootstrap. No mock authority or local fallback.
use crate::{
    room_log::{
        cbfs_archive::{CbfsArchive, RecoveryLimits},
        cbqs::CbqsOwnerLog,
        intent::IntentJournal,
        ownership::WriterRegistry,
        runtime::{OwnerRuntime, RuntimeLimits, WorkerIncarnation},
    },
    ServerConfig,
};
use anyhow::{ensure, Context, Result};
use cbfs_cli::{
    auth::CowboyClient,
    authenticated_state::TrustedCheckpoint,
    config::CliMode,
    connect::{create_volume, open_volume, VolumeContext},
};
use cbfs_registry_proto::AccessMode;
use cbqs_client::{
    chain_view_v2, AuthenticatedStreamViewV2, CheckpointTrustV2, PinnedBrokerEndpoint,
    SessionConfig,
};
#[cfg(feature = "room-key-demo")]
use cbssd::room_deployment::CompiledRoomDeployment;
use cowboy_protocol_codec::{cbqs_v2 as wire, Address};
use ed25519_dalek::{Signer, SigningKey};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::Read,
    net::SocketAddr,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const IO_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_BYTES: usize = 4 * 1024 * 1024;
const SAFETY_MS: u64 = 30_000;
const fn default_max_rooms_per_wallet() -> usize {
    100
}
const fn default_max_pending_rooms_per_wallet() -> usize {
    4
}

#[cfg(feature = "room-key-demo")]
mod initial_room;
#[cfg(feature = "room-key-demo")]
pub use initial_room::{
    activate_initial_room, probe_initial_room, BrowserInitialRoom, InitialRoomDemo,
};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub worker_dir: PathBuf,
    cbfs_state_dir: PathBuf,
    rpc_url: String,
    trusted_checkpoint_file: PathBuf,
    owner_address: String,
    chain_instance_id: String,
    stream_id: String,
    provider_address: String,
    admin_key_file: PathBuf,
    broker_url: String,
    broker_pin: Option<BrokerPin>,
    archive_volume: String,
    control_volume: String,
    api_key_file: PathBuf,
    http_addr: SocketAddr,
    http_origins: Vec<String>,
    public_ws_url: String,
    #[serde(default = "default_max_rooms_per_wallet")]
    max_rooms_per_wallet: usize,
    #[serde(default = "default_max_pending_rooms_per_wallet")]
    max_pending_rooms_per_wallet: usize,
    /// This first entrypoint has a bounded process lifetime, not renewal.
    session_seconds: u64,
}

/// Browser room-key coordinator. It owns only hosted service configuration;
/// generated room keys and member signing keys remain in the browser.
#[cfg(feature = "room-key-demo")]
#[derive(Clone)]
pub struct HostedRoomKeys {
    config: Config,
}

#[cfg(feature = "room-key-demo")]
impl HostedRoomKeys {
    pub fn new(config: &Config) -> Self {
        Self {
            config: config.clone(),
        }
    }

    /// The deployment service ID is also the Cowchat session server ID. This
    /// lets native jobs reject a valid session challenge from another service.
    pub fn service_id(&self) -> Result<[u8; 32]> {
        Ok(CompiledRoomDeployment::compiled()?.service_id())
    }

    pub fn public_ws_url(&self) -> &str {
        &self.config.public_ws_url
    }

    pub fn wallet_room_limits(&self) -> (usize, usize) {
        (
            self.config.max_rooms_per_wallet,
            self.config.max_pending_rooms_per_wallet,
        )
    }

    async fn control_root(&self) -> Result<[u8; 32]> {
        let auth = authority(&self.config).await?;
        let ctx = volume_with_access(
            &self.config,
            &auth,
            &self.config.control_volume,
            AccessMode::ReadOnly,
            false,
        )
        .await?;
        let root = ctx.volume.manifest_root().0;
        ctx.close().await;
        Ok(root)
    }

    pub async fn initial_context(
        &self,
        owner: Address,
        room_id: String,
    ) -> Result<serde_json::Value> {
        let root = self.control_root().await?;
        cbssd::room_deployment::CompiledRoomDeployment::compiled()?
            .initial_browser_context(owner, room_id, root)
    }

    pub async fn successor_context(
        &self,
        previous: &cowboy_protocol_codec::room_policy::SignedRoomKeyPolicyV1,
        removed_member: Address,
    ) -> Result<serde_json::Value> {
        let root = self.control_root().await?;
        cbssd::room_deployment::CompiledRoomDeployment::compiled()?.successor_browser_context(
            previous,
            removed_member,
            root,
        )
    }

    pub async fn attest_setup(&self, signed_setup: &[u8]) -> Result<Vec<String>> {
        Ok(cbssd::room_deployment::CompiledRoomDeployment::compiled()?
            .attest_browser_setup(signed_setup)
            .await?
            .into_iter()
            .map(hex::encode)
            .collect())
    }

    /// Reject stale/conflicting preparations before they enter the owner log.
    /// An exact publication from a lost-ack attempt is accepted for retry.
    pub(crate) async fn preflight(&self, input: &BrowserInitialRoom) -> Result<()> {
        initial_room::preflight_initial_room(&self.config, input).await
    }

    /// Reserve the room in the owner log. Holds the owner-write lock only for
    /// this local append; pass `submit = false` to resume a room whose records
    /// are already durable. Kept separate from [`Self::finalize`] so the caller
    /// releases the lock across the CBSS ceremony.
    pub(crate) async fn stage(
        &self,
        runtime: &mut OwnerRuntime,
        input: &BrowserInitialRoom,
        submit: bool,
    ) -> Result<()> {
        initial_room::stage_initial_room(runtime, input, submit).await
    }

    /// Publish and finalize the prepared epoch against CBSS. Touches no owner
    /// runtime, so the caller runs it with the owner-write lock released.
    pub(crate) async fn finalize(
        &self,
        input: BrowserInitialRoom,
    ) -> Result<initial_room::PreparedInitialRoom> {
        initial_room::finalize_initial_room(&self.config, input).await
    }

    /// Commit the finalized epoch into the owner log under the owner-write lock.
    pub(crate) async fn commit(
        &self,
        runtime: &mut OwnerRuntime,
        prepared: &initial_room::PreparedInitialRoom,
    ) -> Result<()> {
        initial_room::commit_initial_room(runtime, prepared).await
    }

    pub fn open_context(
        &self,
        policy: &cowboy_protocol_codec::room_policy::SignedRoomKeyPolicyV1,
        grant: &cowboy_protocol_codec::room_release::SignedRoomKeyGrantV1,
        custody: &[u8],
    ) -> Result<serde_json::Value> {
        cbssd::room_deployment::CompiledRoomDeployment::compiled()?
            .browser_open_context(policy, grant, custody)
    }

    pub async fn relay_open(&self, request: &[u8]) -> Result<Vec<String>> {
        Ok(cbssd::room_deployment::CompiledRoomDeployment::compiled()?
            .relay_browser_open(request)
            .await?
            .into_iter()
            .map(hex::encode)
            .collect())
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrokerPin {
    address: SocketAddr,
    certificate_der_file: PathBuf,
}

fn fixed<const N: usize>(text: &str) -> Result<[u8; N]> {
    let bytes =
        hex::decode(text.strip_prefix("0x").unwrap_or(text)).context("invalid hex identity")?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid identity length"))
}

fn read_file(path: &Path, secret: bool, limit: u64) -> Result<Zeroizing<Vec<u8>>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.len() <= limit,
        "invalid or oversized configuration file"
    );
    if secret {
        ensure!(
            meta.permissions().mode() & 0o077 == 0
                && meta.nlink() == 1
                && meta.uid() == unsafe { libc::geteuid() },
            "credential files must be private, owned, and not hardlinked"
        );
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "configuration file grew beyond its bound"
    );
    Ok(bytes)
}

fn private_directory(path: &Path) -> Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir()
            && !meta.file_type().is_symlink()
            && meta.permissions().mode() & 0o077 == 0
            && meta.uid() == unsafe { libc::geteuid() },
        "worker directories must be private owned directories"
    );
    Ok(())
}

/// Held from preflight until the process stops, including initialization. A
/// second process cannot share the worker's CBFS pending journals or local DB.
pub struct StateGuard(File);
impl Drop for StateGuard {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = serde_json::from_slice(&read_file(path, false, 64 * 1024)?)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        for path in [
            &self.worker_dir,
            &self.cbfs_state_dir,
            &self.trusted_checkpoint_file,
            &self.admin_key_file,
            &self.api_key_file,
        ] {
            ensure!(path.is_absolute(), "hosted paths must be absolute");
        }
        ensure!(
            (120..=3600).contains(&self.session_seconds),
            "session_seconds must be 120..=3600"
        );
        ensure!(
            self.worker_dir.join("server.sock").as_os_str().len() < 100,
            "worker directory is too long for a Unix socket"
        );
        ensure!(
            self.archive_volume != self.control_volume,
            "archive/control volume names must differ"
        );
        cbfs_cli::state::validate_volume_name(&self.archive_volume)?;
        cbfs_cli::state::validate_volume_name(&self.control_volume)?;
        fixed::<20>(&self.owner_address)?;
        fixed::<20>(&self.provider_address)?;
        fixed::<32>(&self.chain_instance_id)?;
        fixed::<32>(&self.stream_id)?;
        for (value, scheme) in [(&self.rpc_url, "http"), (&self.broker_url, "wss")] {
            let url = reqwest::Url::parse(value)?;
            ensure!(
                (url.scheme() == scheme || (scheme == "http" && url.scheme() == "https"))
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none(),
                "invalid hosted endpoint"
            );
        }
        if let Some(pin) = &self.broker_pin {
            ensure!(
                pin.certificate_der_file.is_absolute(),
                "TLS certificate path must be absolute"
            );
            PinnedBrokerEndpoint::new(
                &self.broker_url,
                pin.address,
                &read_file(&pin.certificate_der_file, false, 16 * 1024)?,
            )
            .map_err(|_| anyhow::anyhow!("invalid pinned broker endpoint"))?;
        }
        ensure!(
            self.http_addr.port() != 0,
            "hosted HTTP port must be explicit"
        );
        let public_ws = reqwest::Url::parse(&self.public_ws_url)?;
        let loopback = public_ws.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
        ensure!(
            (public_ws.scheme() == "wss" || (public_ws.scheme() == "ws" && loopback))
                && public_ws.host_str().is_some()
                && public_ws.username().is_empty()
                && public_ws.password().is_none()
                && public_ws.path() == "/ws"
                && public_ws.query().is_none()
                && public_ws.fragment().is_none(),
            "public_ws_url must be wss://, or ws:// on loopback, with path /ws"
        );
        ensure!(
            (1..=10_000).contains(&self.max_rooms_per_wallet)
                && (1..=100).contains(&self.max_pending_rooms_per_wallet),
            "invalid per-wallet room limits"
        );
        let key = read_file(&self.api_key_file, true, 4096)?;
        ensure!(
            !std::str::from_utf8(&key)?.trim().is_empty(),
            "hosted API key must already exist"
        );
        Ok(())
    }

    /// Call before constructing Tokio or any other threads. The CLI sets the
    /// SDK environment to these private directories while this guard is held.
    pub fn prepare_state(&self) -> Result<StateGuard> {
        private_directory(&self.worker_dir)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.worker_dir.join("worker.lock"))?;
        let meta = file.metadata()?;
        ensure!(
            meta.is_file()
                && meta.nlink() == 1
                && meta.permissions().mode() & 0o077 == 0
                && meta.uid() == unsafe { libc::geteuid() },
            "unsafe worker lock file"
        );
        fs2::FileExt::try_lock_exclusive(&file)
            .context("another process owns this worker directory")?;
        let guard = StateGuard(file);
        private_directory(&self.worker_dir.join("cbfs-pending"))?;
        private_directory(&self.worker_dir.join("cbfs-path-tags"))?;
        ensure!(
            std::fs::canonicalize(&self.worker_dir)?
                != std::fs::canonicalize(&self.cbfs_state_dir)?,
            "worker and CBFS credential directories must differ"
        );
        Ok(guard)
    }

    fn check_environment(&self) -> Result<()> {
        for (name, subdir) in [
            ("CBFS_PENDING_DIFF_DIR", "cbfs-pending"),
            ("CBFS_PATH_TAG_KEY_DIR", "cbfs-path-tags"),
        ] {
            ensure!(
                std::env::var_os(name).as_deref() == Some(self.worker_dir.join(subdir).as_os_str()),
                "hosted SDK state environment was not isolated before startup"
            );
        }
        Ok(())
    }

    pub fn server_config(&self) -> ServerConfig {
        ServerConfig {
            socket_path: self.worker_dir.join("server.sock"),
            tcp_addr: None,
            http_addr: Some(self.http_addr.to_string()),
            db_path: self.worker_dir.join("auth.sqlite"),
            auth_key_path: self.api_key_file.clone(),
            no_auth: false,
            allow_keyless_local: false,
            allow_private_webhooks: false,
            http_signup_enabled: false,
            http_admin_secret: None,
            http_allowed_origins: self.http_origins.clone(),
            trusted_proxy_ips: vec![],
            blob_idle_expiry_seconds: crate::server::DEFAULT_BLOB_IDLE_EXPIRY_SECS,
        }
    }
}

struct Authority {
    cbfs: CowboyClient,
    checkpoint: TrustedCheckpoint,
    view: AuthenticatedStreamViewV2,
    admin: SigningKey,
}

fn now_ms() -> Result<u64> {
    u64::try_from(chrono::Utc::now().timestamp_millis()).context("clock predates Unix epoch")
}

async fn authority(config: &Config) -> Result<Authority> {
    config.check_environment()?;
    let cbfs = CowboyClient::load(&config.cbfs_state_dir, Some(&config.rpc_url))?;
    let owner = fixed::<20>(&config.owner_address)?;
    ensure!(
        cbfs.delegation.wallet_address.as_bytes() == &owner,
        "CBFS credentials do not belong to configured owner"
    );
    let secret = read_file(&config.admin_key_file, true, 256)?;
    let seed = Zeroizing::new(fixed::<32>(std::str::from_utf8(&secret)?.trim())?);
    let admin = SigningKey::from_bytes(&seed);
    let anchor = read_file(&config.trusted_checkpoint_file, false, 1024 * 1024)?;
    let checkpoint = TrustedCheckpoint::from_bytes(anchor.to_vec())?;
    let stream = fixed(&config.stream_id)?;
    let provider = Address::from_bytes(fixed(&config.provider_address)?);
    let instance = fixed(&config.chain_instance_id)?;
    let actor = format!(
        "0x{}",
        hex::encode(chain_view_v2::STREAM_REGISTRY_SYSTEM_ACTOR)
    );
    let body = serde_json::json!({"checkpoint_height":checkpoint.height(),"bundle_version":2,
        "claims":[{"actor":actor,"logical_key_hex":format!("0x{}",hex::encode(chain_view_v2::stream_key(&stream)))},
                  {"actor":actor,"logical_key_hex":format!("0x{}",hex::encode(chain_view_v2::provider_key(&provider)))}]});
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut response = client
        .post(format!(
            "{}/proof/finalized-state",
            config.rpc_url.trim_end_matches('/')
        ))
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    let mut proof = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            proof.len().saturating_add(chunk.len()) <= 32 * 1024 * 1024,
            "finalized proof exceeds bound"
        );
        proof.extend_from_slice(&chunk);
    }
    let view = chain_view_v2::authenticate(&anchor, &proof, &stream, &provider, &instance)?;
    ensure!(
        view.chain_id() == cbfs.delegation.chain_id && view.stream().owner.as_bytes() == &owner,
        "proved chain/stream owner mismatch"
    );
    ensure!(
        view.stream().status == wire::StreamStatusV2::Active,
        "proved stream is not active"
    );
    ensure!(
        view.stream().admin_key.algorithm == wire::SigningKeyAlgorithmV2::Ed25519
            && view.stream().admin_key.key_bytes == admin.verifying_key().to_bytes(),
        "admin key does not match proved stream authority"
    );
    ensure!(
        view.provider()
            .endpoints
            .iter()
            .any(|endpoint| endpoint == config.broker_url.as_bytes()),
        "broker URL not present in proved provider record"
    );
    let now = now_ms()?;
    ensure!(
        view.target_timestamp_ms() <= now.saturating_add(30_000)
            && now.saturating_sub(view.target_timestamp_ms()) <= 120_000,
        "finalized proof is not recent"
    );
    ensure!(
        cbfs.delegation.expires_at_ms > now.saturating_add(120_000),
        "CBFS delegation is near expiry"
    );
    Ok(Authority {
        cbfs,
        checkpoint,
        view,
        admin,
    })
}

async fn volume_with_access(
    config: &Config,
    auth: &Authority,
    name: &str,
    access: AccessMode,
    mount: bool,
) -> Result<VolumeContext> {
    // A hosted writer is a long-lived owner session, using the existing mount
    // token lifetime with the same ReadWrite grant. No renewal or widening.
    let ctx = open_volume(
        &config.cbfs_state_dir,
        name,
        CliMode::Cowboy,
        Some(&config.rpc_url),
        access,
        mount,
        5,
        Some(&auth.checkpoint),
    )
    .await?;
    let meta = ctx
        .cached_open_response
        .as_ref()
        .context("missing Cowboy volume metadata")?;
    ensure!(
        ctx.http_config.is_some()
            && meta.owner_address == auth.cbfs.delegation.wallet_address
            && meta.visibility == cbfs_registry_proto::Visibility::Private,
        "hosted volume must be private and owned by the stream owner"
    );
    ensure!(
        ctx.initial_token_expires_at_ms
            .is_some_and(|expiry| expiry > now_ms().unwrap_or(u64::MAX).saturating_add(120_000)),
        "CBFS attachment is near expiry"
    );
    Ok(ctx)
}

async fn volume(config: &Config, auth: &Authority, name: &str) -> Result<VolumeContext> {
    volume_with_access(config, auth, name, AccessMode::ReadWrite, true).await
}

/// Provisioning never adopts an existing empty volume. A partial failure keeps
/// its created volume for diagnosis; choose new names rather than reset it.
pub async fn initialize(config: &Config, reserve_wei: u128, k: u8, m: u8) -> Result<()> {
    ensure!(
        reserve_wei > 0 && k > 0 && m > 0 && u16::from(k) + u16::from(m) <= 255,
        "invalid provisioning reserve/layout"
    );
    let auth = authority(config).await?;
    for name in [&config.control_volume, &config.archive_volume] {
        ensure!(
            cbfs_cli::state::try_load(&config.cbfs_state_dir, name)?.is_none(),
            "provisioning requires unused volume names"
        );
    }
    let instance = auth.view.chain_instance_id();
    let stream = auth.view.stream().stream_id;
    for (name, control) in [
        (&config.control_volume, true),
        (&config.archive_volume, false),
    ] {
        let created = create_volume(
            &config.cbfs_state_dir,
            name,
            &[],
            k,
            m,
            cbfs_types::Visibility::Private,
            CliMode::Cowboy,
            Some(&config.rpc_url),
            reserve_wei,
            true,
            Some(&auth.checkpoint),
        )
        .await?;
        let ctx = volume(config, &auth, name).await?;
        ensure!(
            ctx.volume.volume_id().0 == created.volume_id,
            "created volume identity changed"
        );
        if control {
            WriterRegistry::initialize_new_volume(
                ctx.volume,
                ctx.auth_store,
                ctx.registry,
                instance,
                stream,
                IO_TIMEOUT,
            )
            .await?;
        } else {
            CbfsArchive::initialize_new_volume(
                ctx.volume,
                ctx.auth_store,
                ctx.registry,
                instance,
                stream,
                MAX_BYTES,
                IO_TIMEOUT,
            )
            .await?;
        }
    }
    Ok(())
}

async fn attach(
    config: &Config,
    auth: &Authority,
    worker: &WorkerIncarnation,
    epoch: u64,
    expiry: u64,
) -> Result<CbqsOwnerLog> {
    let now = now_ms()?;
    let mut grant = wire::StreamGrantV2 {
        version: wire::CBQS_VERSION_V2,
        chain_instance_id: auth.view.chain_instance_id(),
        stream_id: auth.view.stream().stream_id,
        authorization_generation: auth.view.stream().authorization_generation,
        policy_epoch: epoch,
        grant_nonce: rand::random(),
        holder_signing_key: wire::SigningPublicKeyV2 {
            algorithm: wire::SigningKeyAlgorithmV2::Ed25519,
            key_bytes: worker.holder().verifying_key().to_bytes(),
        },
        verbs: wire::CBQS_V2_VERB_APPEND
            | wire::CBQS_V2_VERB_REPLAY
            | wire::CBQS_V2_VERB_CONSUME
            | wire::CBQS_V2_VERB_LANE_ADMIN,
        lane_scope: wire::LaneScopeV2::Any,
        group_scope: wire::GroupScopeV2::Any,
        not_before_ms: now.saturating_sub(1000),
        expires_at_ms: expiry,
        max_message_bytes: 262_144,
        max_append_bytes_per_sec: 1_048_576,
        signature: wire::CbqsSignatureV2([0; 64]),
    };
    grant.signature = wire::CbqsSignatureV2(
        auth.admin
            .sign(&cowboy_protocol_codec::keccak256(
                &wire::stream_grant_signing_bytes_v2(&grant),
            ))
            .to_bytes(),
    );
    let (fence, session) = if let Some(pin) = &config.broker_pin {
        let endpoint = PinnedBrokerEndpoint::new(
            &config.broker_url,
            pin.address,
            &read_file(&pin.certificate_der_file, false, 16 * 1024)?,
        )
        .map_err(|_| anyhow::anyhow!("invalid pinned broker endpoint"))?;
        (
            endpoint
                .connect_socket(&config.broker_url)
                .await
                .map_err(|_| anyhow::anyhow!("pinned broker connection failed"))?,
            endpoint
                .connect_socket(&config.broker_url)
                .await
                .map_err(|_| anyhow::anyhow!("pinned broker connection failed"))?,
        )
    } else {
        let url = reqwest::Url::parse(&config.broker_url)?;
        let addresses: Vec<_> = tokio::net::lookup_host((
            url.host_str().context("missing broker host")?,
            url.port_or_known_default().unwrap_or(443),
        ))
        .await?
        .map(|a| a.ip())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
        (
            cbqs_client::transport_v2::connect_checked(&config.broker_url, &addresses)
                .await
                .map_err(|_| anyhow::anyhow!("checked broker connection failed"))?,
            cbqs_client::transport_v2::connect_checked(&config.broker_url, &addresses)
                .await
                .map_err(|_| anyhow::anyhow!("checked broker connection failed"))?,
        )
    };
    Ok(CbqsOwnerLog::attach(
        fence,
        session,
        SessionConfig {
            broker_url: config.broker_url.clone(),
            grant,
            holder: worker.holder().clone(),
            handshake_timeout: Duration::from_secs(30),
            checkpoints: CheckpointTrustV2::ChainProvider(Box::new(auth.view.provider().clone())),
        },
        |bytes| {
            auth.admin
                .sign(&cowboy_protocol_codec::keccak256(bytes))
                .to_bytes()
        },
        now_ms()?,
    )
    .await?)
}

/// The caller supplies the one expected epoch. Never auto-increment/retry a
/// losing CAS. Recover fully before the caller constructs any listener.
pub async fn recover(config: &Config, expected_epoch: u64) -> Result<(OwnerRuntime, Instant)> {
    ensure!(expected_epoch < u64::MAX - 1, "expected epoch is exhausted");
    let auth = authority(config).await?;
    let owner = format!("0x{}", hex::encode(auth.view.stream().owner.as_bytes()));
    let instance = auth.view.chain_instance_id();
    let stream = auth.view.stream().stream_id;
    let journal = IntentJournal::open(
        &config.worker_dir.join("intents.sqlite"),
        owner.clone(),
        instance,
        stream,
        100,
        MAX_BYTES,
    )?;
    let ctx = volume(config, &auth, &config.control_volume).await?;
    let mut expiry = now_ms()?
        .saturating_add(config.session_seconds * 1000)
        .min(auth.cbfs.delegation.expires_at_ms)
        .min(
            ctx.initial_token_expires_at_ms
                .context("missing token expiry")?,
        );
    let mut registry = WriterRegistry::open(
        ctx.volume,
        ctx.auth_store,
        ctx.registry,
        instance,
        stream,
        IO_TIMEOUT,
    )
    .await?;
    let observed_epoch = registry.epoch()?;
    ensure!(observed_epoch == expected_epoch,
        "control epoch mismatch: expected {expected_epoch}, observed {observed_epoch}; no claim attempted");
    let worker = WorkerIncarnation::fresh();
    let allocation = registry
        .claim(expected_epoch, worker.writer_id(), worker.claim_id())
        .await?;
    log::info!(
        "Acquired hosted writer epoch {}; fencing before recovery",
        allocation.epoch()
    );
    let log = tokio::time::timeout(
        IO_TIMEOUT,
        attach(config, &auth, &worker, allocation.epoch(), expiry),
    )
    .await??;
    let writer = worker.bind(allocation, log)?;
    // Open archive AFTER fencing the previous writer, so its final committed
    // root participates in recovery rather than a pre-fence snapshot.
    let ctx = volume(config, &auth, &config.archive_volume).await?;
    expiry = expiry.min(
        ctx.initial_token_expires_at_ms
            .context("missing token expiry")?,
    );
    let archive = CbfsArchive::open(
        ctx.volume,
        ctx.auth_store,
        ctx.registry,
        instance,
        stream,
        MAX_BYTES,
        IO_TIMEOUT,
    )
    .await?;
    let mut runtime = OwnerRuntime::recover(
        owner,
        writer,
        archive,
        journal,
        RuntimeLimits {
            archive: RecoveryLimits {
                max_segments: 10_000,
                max_records: 100_000,
                max_bytes: 256 * 1024 * 1024,
            },
            batch_records: 100,
            replay_bytes: MAX_BYTES,
        },
    )
    .await?;
    let remaining = expiry
        .checked_sub(now_ms()?.saturating_add(SAFETY_MS))
        .context("credentials expired during recovery")?;
    let deadline = Instant::now() + Duration::from_millis(remaining);
    runtime.expire_at(deadline)?;
    Ok((runtime, deadline))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> (tempfile::TempDir, Config) {
        let dir = tempfile::Builder::new()
            .prefix("cc")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let cbfs = dir.path().join("credentials");
        private_directory(&cbfs).unwrap();
        let key = dir.path().join("api.key");
        std::fs::write(&key, "test-only-api-key").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        let config = Config {
            worker_dir: dir.path().join("worker"),
            cbfs_state_dir: cbfs,
            rpc_url: "http://127.0.0.1:1".into(),
            trusted_checkpoint_file: dir.path().join("checkpoint.bin"),
            owner_address: "01".repeat(20),
            chain_instance_id: "02".repeat(32),
            stream_id: "03".repeat(32),
            provider_address: "04".repeat(20),
            admin_key_file: dir.path().join("admin.key"),
            broker_url: "wss://broker.example/ws".into(),
            broker_pin: None,
            archive_volume: "archive".into(),
            control_volume: "control".into(),
            api_key_file: key,
            http_addr: "127.0.0.1:19440".parse().unwrap(),
            http_origins: vec![],
            public_ws_url: "ws://127.0.0.1:19440/ws".into(),
            max_rooms_per_wallet: default_max_rooms_per_wallet(),
            max_pending_rooms_per_wallet: default_max_pending_rooms_per_wallet(),
            session_seconds: 600,
        };
        (dir, config)
    }

    #[test]
    fn preflight_refuses_missing_public_or_linked_credentials_without_creating_state() {
        let (_dir, config) = config();
        config.validate().unwrap();
        std::fs::set_permissions(&config.api_key_file, std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(config.validate().is_err());
        std::fs::set_permissions(&config.api_key_file, std::fs::Permissions::from_mode(0o600))
            .unwrap();
        let saved = config.api_key_file.with_extension("saved");
        std::fs::rename(&config.api_key_file, &saved).unwrap();
        assert!(config.validate().is_err());
        std::os::unix::fs::symlink(&saved, &config.api_key_file).unwrap();
        assert!(config.validate().is_err());
        std::fs::remove_file(&config.api_key_file).unwrap();
        std::fs::hard_link(&saved, &config.api_key_file).unwrap();
        assert!(config.validate().is_err());
        assert!(!config.worker_dir.exists());
    }

    #[test]
    fn worker_state_is_exclusive_and_rejects_symlink_aliases() {
        let (_dir, config) = config();
        let guard = config.prepare_state().unwrap();
        assert!(config.prepare_state().is_err());
        assert!(!config.worker_dir.join("auth.sqlite").exists());
        drop(guard);
        drop(config.prepare_state().unwrap());
        let pending = config.worker_dir.join("cbfs-pending");
        std::fs::remove_dir(&pending).unwrap();
        std::os::unix::fs::symlink(&config.cbfs_state_dir, &pending).unwrap();
        assert!(config.prepare_state().is_err());
    }

    #[test]
    fn hosted_config_has_no_open_auth_or_local_transport_escape() {
        let (_dir, mut config) = config();
        let server = config.server_config();
        assert!(!server.no_auth && !server.allow_keyless_local && !server.http_signup_enabled);
        assert!(server.tcp_addr.is_none());
        assert!(!server.allow_private_webhooks);
        config.broker_url = "ws://broker.example/ws".into();
        assert!(config.validate().is_err());
        config.broker_url = "wss://broker.example/ws".into();
        config.public_ws_url = "ws://chat.example/ws".into();
        assert!(config.validate().is_err());
        config.public_ws_url = "wss://chat.example/ws".into();
        config.max_rooms_per_wallet = 0;
        assert!(config.validate().is_err());
        config.max_rooms_per_wallet = default_max_rooms_per_wallet();
        config.archive_volume = config.control_volume.clone();
        assert!(config.validate().is_err());
    }
}
